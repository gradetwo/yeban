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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use yeban_app::bridge::{self, NewTrackKind};
use yeban_app::engine_host::{EditMark, EngineHost, PublishOutcome};
use yeban_app::export_wav::WavExportOptions;
use yeban_app::input::{Action, InputContext, MixerParam, Resolution, View};
use yeban_app::undo::{ActionOutcome, UiAction, UndoPort};
use yeban_engine::meter::{MeterCollector, MeterFrame};
use yeban_engine::ring::TransportCommand;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::AssetHash;
use yeban_model::container::ProjectArchive;
use yeban_model::ids::EntityId;
use yeban_model::music::CurveType;
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::project::{
    AutomationPoint, AutomationTarget, ClipContent, ClipPlacement, ClipPoolEntry, MAX_BPM, MIN_BPM,
    TrackKind, YebanProjectV1,
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

/// 一条语义行（T-FIX-86③）：角色 / 标签 / 序，**与 GUI 同一套角色词**。
///
/// 角色串取自 `yeban_app::elements::ElementKind::accessible_role()`（GUI 元素注册表的
/// 唯一词表），⛔ 不另造一套。终端没有 OS 无障碍树，这些行是"可被读出的语义清单"
/// （`:a11y` 导出），取舍写在 README。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRow {
    /// 稳定语义 ID（`tui-track-3` 这类；判据按它定位）。
    pub id: String,
    /// 无障碍角色（`list` / `list-item` / `groupbox` …）。
    pub role: &'static str,
    /// 人类可读标签（与界面上显示的文本同源）。
    pub label: String,
    /// 在同角色兄弟里的序（0 起）。
    pub index: usize,
}

/// 工程信息面读数（T-FIX-86①②）：契约版本 + 960 PPQ 时钟 + EntityId + 规模。
///
/// 每个字段都来自**模型自己**：`YebanProjectV1` 的版本字段、`yeban_model::ids::PPQ`、
/// 实体身份、`App` 的 tick 换算（与时间码/吸附共用同一个 `ticks_per_beat`）。
/// ⛔ 不写死 960 以外的数、不从文件名猜版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    /// 工程标题。
    pub title: String,
    /// 工程身份（`YebanProjectV1::id`，ULID 文本）。
    pub id: String,
    /// 文档 schema 版本。
    pub schema_version: u32,
    /// 最低读取器版本。
    pub min_reader_version: u32,
    /// 写入器版本串。
    pub writer_version: String,
    /// `check_readable` 的结论（`Err` 带原文；`:info` 照实写）。
    pub readable: Result<(), String>,
    /// 每四分音符 tick 数（模型常量 `yeban_model::ids::PPQ`）。
    pub ppq: u64,
    /// 每拍 tick 数（按拍号分母换算）。
    pub ticks_per_beat: u64,
    /// 每小节 tick 数。
    pub bar_ticks: u64,
    /// 拍号分子。
    pub numerator: u8,
    /// 拍号分母。
    pub denominator: u8,
    /// 工程总小节数。
    pub total_bars: usize,
    /// 工程总长（tick）。
    pub length_ticks: u64,
    /// 音轨数（含主总线）。
    pub tracks: usize,
    /// 片段数（池条目）。
    pub clips: usize,
    /// 容器路径（`None` = 内存里的演示工程）。
    pub source: Option<String>,
    /// 选中轨（身份 + 名）。
    pub selected_track: Option<(String, String)>,
    /// 小节光标（0 起）。
    pub cursor_bar: usize,
}

impl ProjectInfo {
    /// 工程信息面文本（`:info` 的四行；判据与界面共用这一份）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let contract = match &self.readable {
            Ok(()) => "可读（check_readable 通过）".to_owned(),
            Err(reason) => format!("不可读（{reason}）"),
        };
        let source = self
            .source
            .clone()
            .unwrap_or_else(|| "内存演示工程（尚未落盘）".to_owned());
        let selected = self.selected_track.as_ref().map_or_else(
            || "未选中轨".to_owned(),
            |(id, name)| format!("轨「{name}」 {id}"),
        );
        vec![
            format!("工程「{}」 · 身份 {} · 容器 {source}", self.title, self.id),
            format!(
                "契约 schema {} · minReader {} · writer {} · {contract}",
                self.schema_version, self.min_reader_version, self.writer_version
            ),
            format!(
                "时钟 {} PPQ · 每拍 {} tick · 每小节 {} tick · {}/{} · 长度 {} 小节 / {} tick",
                self.ppq,
                self.ticks_per_beat,
                self.bar_ticks,
                self.numerator,
                self.denominator,
                self.total_bars,
                self.length_ticks
            ),
            format!(
                "规模 {} 轨 · {} 片段 · 选中 {selected} · 光标第 {} 小节",
                self.tracks,
                self.clips,
                self.cursor_bar + 1
            ),
        ]
    }
}

/// 鼓机生成请求（`yeban_generate_drum_pattern` 的参数面；缺省与 GUI / MCP 同口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumRequest {
    /// 风格预置 id（`yeban_theory::factory_drum_style_ids()` 之一）。
    pub style: String,
    /// 生成小节数（1..=16）。
    pub bars: u32,
    /// 复杂度旋钮（0..=100，密度/鬼音）。
    pub complexity: u8,
    /// 强度旋钮（0..=100，力度层级）。
    pub intensity: u8,
    /// 摇摆千分比（500..=1000）；`None` = 用风格预置定值。
    pub swing_permille: Option<u16>,
    /// 种子（鬼音哈希的盐；同种子同参数逐字节可复现）。
    pub seed: u64,
    /// 是否在最后一小节落 fill。
    pub fill: bool,
}

impl Default for DrumRequest {
    fn default() -> Self {
        Self {
            style: String::new(),
            bars: 4,
            complexity: 50,
            intensity: 80,
            swing_permille: None,
            seed: 0,
            fill: true,
        }
    }
}

impl DrumRequest {
    /// 范围校验（与 MCP `plan_generate_drum_pattern` 同一批判据与同一批文案）。
    fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.bars) {
            return Err(format!(
                "`bars` 必须是 1..=16 的整数, 实际收到 {}",
                self.bars
            ));
        }
        if self.complexity > 100 {
            return Err(format!(
                "`complexity` 必须是 0..=100 的整数, 实际收到 {}",
                self.complexity
            ));
        }
        if self.intensity > 100 {
            return Err(format!(
                "`intensity` 必须是 0..=100 的整数, 实际收到 {}",
                self.intensity
            ));
        }
        if let Some(swing) = self.swing_permille
            && !(500..=1000).contains(&swing)
        {
            return Err(format!(
                "`swingPermille` 必须是 500..=1000 的整数, 实际收到 {swing}"
            ));
        }
        Ok(())
    }
}

/// 一次诊断包导出的读数（命令回执与测试用；字段与 `yeban_diagnostics::BundleReport` 同名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReceipt {
    /// 落盘的 `.zip` 路径。
    pub path: PathBuf,
    /// 压缩包字节数。
    pub bytes: u64,
    /// 压缩包自身的 `sha256`。
    pub sha256: String,
    /// 包内条目名（含 `MANIFEST.txt` 自身；顺序即写出顺序）。
    pub entries: Vec<String>,
}

/// GUI 电平心跳周期（`crates/yeban-app/src/main.rs` 的 `HEARTBEAT_PERIOD_MS = 16`
/// ⇒ 62.5 Hz，不低于 60 Hz）：TUI 主循环用**同一个数**做轮询超时与 tick 闸门。
///
/// ⛔ 不从 `yeban-app` 导：那个常量住在 bin crate 的 `main.rs`，库侧没有它；
/// 数值口径相同（判据 `meter_heartbeat_matches_the_gui_period` 钉住 16 ms）。
pub const METER_HEARTBEAT_MS: u64 = 16;

/// GUI 侧那个常量的**引用副本**（`crates/yeban-app/src/main.rs` 的
/// `HEARTBEAT_PERIOD_MS = 16`）：判据用它做等式比较，而不是在判据里再写一个 `16`
/// （两个 16 各自写一遍就没人保证它们还是同一个口径）。
pub const HEARTBEAT_PERIOD_MS_REFERENCE: u64 = 16;

/// 电平保持的每跳衰减系数（主总线底栏与逐轨电平共用同一个数）。
const METER_DECAY: f32 = 0.92;

/// 低于这个线性幅度就当作静音（≈ -80 dBFS），不保留无意义的尾巴。
const METER_FLOOR: f32 = 1.0e-4;

/// 引擎状态读数（`yeban_query_engine_state` 对应的 TUI 面）。
///
/// 每个字段都从**既有**读数来：工程采样率取 `project.audio_config`；引擎侧取
/// `EngineHost::current_snapshot()`（快照自己的 `sample_rate` / `block_frames` /
/// `channels`）；设备取 `EngineHost::device_name()` / `device_active()`；走带取
/// `EngineHost::transport()`。⛔ 不新造第二套状态源、不从工程推算引擎读数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    /// 工程声明的采样率（Hz）。
    pub project_sample_rate: u32,
    /// 引擎快照的采样率（Hz）；没有引擎时 `None`。
    pub engine_sample_rate: Option<u32>,
    /// 引擎快照的每块帧数；没有引擎时 `None`。
    pub block_frames: Option<u32>,
    /// 引擎快照的通道数；没有引擎时 `None`。
    pub channels: Option<u16>,
    /// 设备腿的显示名（TUI 默认不开设备腿 ⇒ `None`）。
    pub device_name: Option<String>,
    /// 设备腿的采样率（Hz，协商结果）；没有设备腿时 `None`。
    pub device_sample_rate: Option<u32>,
    /// 设备腿是否活跃。
    pub device_active: bool,
    /// 采样率转换的**如实**说明（转换中带「工程 X Hz → 设备 Y Hz」两侧数值，
    /// 与 `DeviceOpening::status_line` 的 T-FIX-72 口径同一句话）。
    pub conversion: String,
    /// 走带状态。
    pub transport_state: TransportState,
    /// 走带位置（tick）。
    pub position_ticks: u64,
}

/// 设备状态行读数（T-FIX-82④）：设备名 / 设备采样率 / 工程采样率 / 转换状态。
///
/// 四个字段都取**既有**读数：设备名与两侧采样率来自 `EngineHost`（协商结果），
/// 工程率来自 `project.audio_config`。没有设备腿时前两个是 `None`，格式函数
/// 会**如实**写「未开设备腿」而不是编一个数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    /// 设备腿的显示名（没有设备腿 ⇒ `None`）。
    pub device_name: Option<String>,
    /// 协商时的工程采样率（Hz，引擎此刻正在转换的源率；没有设备腿 ⇒ `None`）。
    pub source_rate: Option<u32>,
    /// 设备采样率（Hz，协商结果；没有设备腿 ⇒ `None`）。
    pub device_sample_rate: Option<u32>,
    /// 当前工程声明的采样率（Hz）。
    pub project_sample_rate: u32,
}

impl DeviceStatus {
    /// 状态行文本：`设备 <名> · 设备采样率 N Hz · 工程采样率 M Hz · 转换状态 …`。
    ///
    /// 转换中把**两侧数值**都写出来（`采样率转换: 工程 M Hz → 设备 N Hz`），
    /// 与 `DeviceOpening::status_line` 的 T-FIX-72 口径同一句话；两侧一致时
    /// 明写「无采样率转换」，⛔ 不留下一个可以随便解读的空格。
    #[must_use]
    pub fn line(&self) -> String {
        let project = self.project_sample_rate;
        match (&self.device_name, self.device_sample_rate) {
            (Some(name), Some(device)) => {
                let conversion = match self.source_rate {
                    Some(source) if source != device => {
                        format!("采样率转换: 工程 {source} Hz → 设备 {device} Hz")
                    }
                    Some(source) => {
                        format!("无采样率转换（工程 {source} Hz 与设备率一致）")
                    }
                    None => "无采样率转换（协商未报告源率）".to_owned(),
                };
                format!(
                    "设备 {name} · 设备采样率 {device} Hz · 工程采样率 {project} Hz · 转换状态 {conversion}"
                )
            }
            _ => format!(
                "设备 未开设备腿（TUI 走墙钟泵送） · 设备采样率 — · 工程采样率 {project} Hz · \
                 转换状态 无（未开设备腿）"
            ),
        }
    }
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
    /// 自动化泳道（T-FIX-76 第 4 项）：`selected` 是按 `(tick, 点身份)` 排序后的下标。
    ///
    /// 与 `yeban_edit_automation` 的**读**同一入口（`YebanProjectV1::automation_value_at`
    /// 是唯一求值处，这里不排序采样点、不做插值），**写**同一条 `Op::SetAutomationPoint`
    /// （可撤销；同 tick 重复写入更新同一个点，见 [`App::set_automation_value`]）。
    Automation {
        /// 泳道所属音轨（与 `target.track_id()` 恒相等，冗余一份供渲染免解析）。
        track_id: EntityId,
        /// 泳道目标（唯一寻址）。
        target: AutomationTarget,
        /// 选中点的下标。
        selected: usize,
    },
}

/// 自动化视图能打开的泳道种类（批 5：`AutomationTarget` 的**全部六个**变体）。
///
/// ⛔ 不是"TUI 自己定义的一套泳道清单"：每个变体都是 [`AutomationTarget`] 的一对一
/// 投影，单位 / 值域 / 存在性全部由模型派生
/// （[`AutomationTarget::nominal_unit`] / [`AutomationTarget::nominal_domain`] /
/// [`AutomationTarget::validate_against`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationKind {
    /// 音轨音量（dB）。
    Volume,
    /// 音轨声相（-1.0..=1.0）。
    Pan,
    /// 发送增益（dB；`edge_id` = 路由图里的那条发送边）。
    Send {
        /// 发送所对应的路由边。
        edge_id: EntityId,
    },
    /// 设备参数（原生单位；值域由泳道声明或曲线自适应）。
    Device {
        /// 设备链下标。
        slot_index: usize,
        /// 参数下标。
        param_index: usize,
    },
    /// 宏（归一化 0.0..=1.0）。
    Macro {
        /// 宏下标。
        macro_index: usize,
    },
    /// MIDI CC（原始 7 位值 0..=127）。
    MidiCc {
        /// 控制器号。
        controller: u8,
    },
}

impl AutomationKind {
    /// 目标构造（与 MCP `yeban_edit_automation` 的 `lane` 实参同一目标）。
    #[must_use]
    pub const fn target(self, track_id: EntityId) -> AutomationTarget {
        match self {
            Self::Volume => AutomationTarget::TrackVolume { track_id },
            Self::Pan => AutomationTarget::TrackPan { track_id },
            Self::Send { edge_id } => AutomationTarget::SendGain { track_id, edge_id },
            Self::Device {
                slot_index,
                param_index,
            } => AutomationTarget::DeviceParam {
                track_id,
                slot_index,
                param_index,
            },
            Self::Macro { macro_index } => AutomationTarget::Macro {
                track_id,
                macro_index,
            },
            Self::MidiCc { controller } => AutomationTarget::MidiCc {
                track_id,
                controller,
            },
        }
    }

    /// 泳道**种类**短名（命令用法与日志前缀）。
    ///
    /// ⛔ 不是泳道的人读名 —— 那个走 [`App::automation_lane_name`]（与 GUI 同一命名口径，
    /// 设备参数会显示参数名、宏会显示宏名）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Volume => "音量",
            Self::Pan => "声相",
            Self::Send { .. } => "发送增益",
            Self::Device { .. } => "设备参数",
            Self::Macro { .. } => "宏",
            Self::MidiCc { .. } => "MIDI CC",
        }
    }

    /// 一次微调的步长。
    ///
    /// 音量 / 发送保持既有的 0.5 dB、声相 / 宏保持 0.05；CC 是整数域 ⇒ 1；
    /// 设备参数的值域模型不声明（`nominal_domain` 返回 `None`）⇒ 有泳道声明时取
    /// 值域的 1%，否则 0.01（⛔ 不替模型编一个值域）。
    #[must_use]
    pub fn step(self, domain: Option<(f32, f32)>) -> f32 {
        match self {
            Self::Volume | Self::Send { .. } => 0.5,
            Self::Pan | Self::Macro { .. } => 0.05,
            Self::MidiCc { .. } => 1.0,
            Self::Device { .. } => {
                domain.map_or(0.01, |(min, max)| ((max - min).abs() / 100.0).max(1e-3))
            }
        }
    }
}

/// 自动化视图在光标 tick 上的三态读数（与 `yeban_edit_automation` 的读回同语义）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutomationReadout {
    /// 查询的 tick（= 小节光标 × 每小节 tick）。
    pub tick: u64,
    /// 唯一求值入口的返回值；`None` = 本泳道没有可用自动化值。
    pub automation_value: Option<f32>,
    /// 目标的静态值（无自动化时的回退）。
    pub static_value: f32,
    /// 实际生效值（自动化优先，否则静态）。
    pub effective_value: f32,
    /// `"automation"` 或 `"static"`。
    pub source: &'static str,
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

/// 主视图的中文名（状态行 / 日志）。
fn view_label(view: View) -> &'static str {
    match view {
        View::Session => "Session",
        View::Arrangement => "编曲",
    }
}

/// 自动化目标 → TUI 泳道种类（六类一一对应；`AutomationTarget` 没有第七个变体）。
fn kind_of(target: AutomationTarget) -> AutomationKind {
    match target {
        AutomationTarget::TrackVolume { .. } => AutomationKind::Volume,
        AutomationTarget::TrackPan { .. } => AutomationKind::Pan,
        AutomationTarget::SendGain { edge_id, .. } => AutomationKind::Send { edge_id },
        AutomationTarget::DeviceParam {
            slot_index,
            param_index,
            ..
        } => AutomationKind::Device {
            slot_index,
            param_index,
        },
        AutomationTarget::Macro { macro_index, .. } => AutomationKind::Macro { macro_index },
        AutomationTarget::MidiCc { controller, .. } => AutomationKind::MidiCc { controller },
    }
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
    /// 主视图（`Session` / `Arrangement`，与 GUI 同一个 [`View`] 枚举）。
    pub view: View,
    /// 轨道高度视图态（`bridge::TrackHeightLayout`：每轨基准 + 全局乘子，
    /// 夹紧只有 `bridge::effective_track_height_px` 一处 —— TUI 不另立边界）。
    height: bridge::TrackHeightLayout,
    /// 会话视图里选中的场景下标。
    pub scene_selected: usize,
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
    /// 最近一次 `:sessions` 列出的已存工程路径（按显示顺序），供 `:sessions open <n>` 按序号打开。
    /// 不另存第二套状态源：来源只有 `yeban_app::settings::recent_projects`。
    session_search: Vec<PathBuf>,
    /// 日志行（最新在尾；底栏显示最后一条）。
    pub log: Vec<String>,
    /// 有未保存修改。
    pub dirty: bool,
    /// 主总线最新峰值（线性幅度，带衰减保持，供底栏电平条）。
    pub master_peak: f32,
    /// 逐轨峰值（同一份 SPSC 链的抽干结果，带衰减保持）。
    track_peaks: BTreeMap<EntityId, f32>,
    /// 是否请求退出（主循环读）。
    pub quit: bool,
    /// 上次导出证据。
    pub last_export: Option<ExportReceipt>,
    /// 已播报过的音频注入修订（避免每跳重复刷日志）。
    audio_revision: u64,
    /// ⭐ P0-7：上一次发布因解码转后台而延后 —— 每跳 `tick` 重试发布，
    /// 就绪（或失败）后清掉（重试细节见 `publish_snapshot`）。
    publish_deferred: bool,
    /// 自动化采样缓存需要换一份工程：构造后的首跳、以及每次 [`Self::sync_from_port`]
    /// 之后置位。`tick` 只在置位这一跳把工程交给 `publish_automation(Some(..))`，
    /// 其余各跳传 `None`（复用 `EngineHost` 的缓存，与 GUI 的 R68 口径一致）。
    automation_project_pending: bool,
}

/// 保存前把当前工程实际触发的样本收进归档（T-FIX-160）。
///
/// 与 `yeban_save_project` 共用 [`yeban_mcp::domain::embed_samples::collect`]：
/// 刷新 `embedded_samples` 表、合并 `added`、剪掉 `pruned`，并保留旧归档里的
/// 提交图谱 / 提案 / 内嵌预制 / 装载留痕。返回缺失样本数（保存仍成功，回执点名）。
fn prepare_archive_for_save(
    project: YebanProjectV1,
    project_path: &Path,
    base: Option<&ProjectArchive>,
) -> Result<(ProjectArchive, usize), String> {
    let assets: BTreeMap<AssetHash, Vec<u8>> = base
        .map(|archive| archive.assets.iter().cloned().collect())
        .unwrap_or_default();
    let collected = yeban_mcp::domain::embed_samples::collect(&project, project_path, &assets)
        .map_err(|error| format!("保存失败：内嵌采样收集失败：{error:?}"))?;
    let yeban_mcp::domain::embed_samples::EmbeddedCollection {
        map,
        added,
        pruned,
        missing,
    } = collected;
    let mut merged = assets;
    for (hash, bytes) in added {
        merged.insert(hash, bytes);
    }
    for hash in pruned {
        merged.remove(&hash);
    }
    let mut project = project;
    project.embedded_samples = map;
    let mut archive = base.cloned().unwrap_or_else(|| ProjectArchive {
        project: project.clone(),
        history_dag: Vec::new(),
        proposals: None,
        assets: Vec::new(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    });
    archive.project = project;
    archive.assets = merged.into_iter().collect();
    Ok((archive, missing.len()))
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
            proposals: None,
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
    /// 成功打开后回写最近工程清单（与 GUI 同落点
    /// `yeban_app::settings::record_recent_project`，`:sessions` 才能
    /// 看到）；`with_demo` / `with_project` 不走这里，不会误记演示工程。
    /// 回写失败不拦打开（`record_recent_project` 按 `save_language`
    /// 同款契约只记一条待发警告）。
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
        let app = Self::assemble(project, Some(archive), Some(path.to_path_buf()))?;
        yeban_app::settings::record_recent_project(path, now_ms());
        Ok(app)
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
            proposals: None,
            assets: Vec::new(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        });
        let source = project_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<demo>".to_owned());
        let shared_project = Rc::new(std::cell::RefCell::new(archive.project.clone()));
        let session = yeban_app::undo::session_from_archive(
            &source,
            shared_project,
            &archive.history_dag,
            now_ms(),
        )?;
        // ⭐ 音频片段必须拿得到**字节**：TUI 独占终端时静默引擎诊断，由日志行统一播报。
        let asset_bytes: BTreeMap<AssetHash, Vec<u8>> = archive.assets.iter().cloned().collect();
        let port = Rc::new(UndoPort::new(session));
        let project = port.project();
        let mut host = EngineHost::new();
        host.set_quiet_diagnostics(true);
        host.set_audio_asset_source(Box::new(move |hash: &AssetHash| {
            asset_bytes.get(hash).cloned()
        }));
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
            view: View::Arrangement,
            height: bridge::TrackHeightLayout::default(),
            scene_selected: 0,
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
            session_search: Vec::new(),
            log: Vec::new(),
            dirty: false,
            master_peak: 0.0,
            track_peaks: BTreeMap::new(),
            quit: false,
            last_export: None,
            audio_revision: 0,
            publish_deferred: false,
            automation_project_pending: true,
        };
        app.refresh_track_ids();
        app.report_audio_issues();
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

    /// 音频注入读数：(注入素材数, 跳过数)。判据入口，界面不直接消费。
    #[must_use]
    pub fn audio_injection_counts(&self) -> (usize, usize) {
        let report = self.host.audio_clip_report();
        (report.injected_assets, report.skipped.len())
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

    /// 当前位置（小节，拍，tick 余数）：小节与拍均从 1 起（与标尺及 GUI 一致），tick 余数从 0 起。
    #[must_use]
    pub fn position_bar_beat(&self) -> (u64, u64, u64) {
        let ticks = self.transport().position_ticks;
        let per_beat = self.ticks_per_beat().max(1);
        let per_bar = self.bar_ticks().max(1);
        (
            ticks / per_bar + 1,
            (ticks % per_bar) / per_beat + 1,
            ticks % per_beat,
        )
    }

    /// 播放头所在小节下标（0 起，供网格标尺与横向跟随对齐 `cursor_bar`）。
    #[must_use]
    pub fn playhead_bar_index(&self) -> usize {
        let per_bar = self.bar_ticks().max(1);
        usize::try_from(self.transport().position_ticks / per_bar).unwrap_or(usize::MAX)
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
        self.retry_deferred_publish();
        self.report_audio_issues();
        let _ = self.host.heartbeat();
        // 自动化采样必须每跳发生（与 GUI 的 `ProductionLoop::tick` 同一条纪律）：
        // 工程缓存只在首跳/编辑后换一份，其余各跳传 `None` 复用缓存；若每跳都传
        // `Some`，16 ms 一跳会克隆整份工程并清空去重表，代价与 GUI 不一致。
        if self.automation_project_pending {
            self.automation_project_pending = false;
            let project = self.project.clone();
            let _ = self.host.publish_automation(Some(&project));
        } else {
            let _ = self.host.publish_automation(None);
        }
        if let Some(collector) = &mut self.collector {
            let mut scratch = [MeterFrame::default(); 128];
            let drained = collector.drain_latest(&mut scratch);
            let master = self.project.master_bus_track_id;
            // 同一份 SPSC 链的抽干结果按节点身份落到两处：主总线进底栏点阵，
            // 其余逐轨进通道电平（⛔ 不旁路轮询、不另开一条近似值来源）。
            for frame in &scratch[..drained] {
                if frame.node == master {
                    self.master_peak = frame.peak.max(self.master_peak * METER_DECAY);
                } else {
                    let peak = self.track_peaks.entry(frame.node).or_insert(0.0);
                    *peak = frame.peak.max(*peak * METER_DECAY);
                }
            }
            if drained == 0 {
                self.master_peak *= METER_DECAY;
            }
            // 没有新帧的轨也照常衰减（画面不会停在上一帧的满格）。
            for peak in self.track_peaks.values_mut() {
                *peak *= METER_DECAY;
            }
            self.track_peaks.retain(|_, peak| *peak > METER_FLOOR);
        }
    }

    /// 把引擎的音频注入读数收成**一条**日志（只在修订变化且确有跳过时说话）。
    fn report_audio_issues(&mut self) {
        let revision = self.host.audio_clip_report().revision;
        if revision == self.audio_revision {
            return;
        }
        self.audio_revision = revision;
        let (injected, skipped) = {
            let report = self.host.audio_clip_report();
            (report.injected_assets, report.skipped.clone())
        };
        if skipped.is_empty() {
            return;
        }
        let mut line = format!("音频片段：注入 {injected} 个 / 跳过 {} 个", skipped.len());
        for skip in &skipped {
            line.push_str(&format!("；{}：{}", skip.asset, skip.reason));
        }
        self.log_line(line);
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
                // T-FIX-244：模型/撤销图已经变更 ⇒ 立即置脏，与引擎发布结局无关。
                // 发布可能 Deferred（后台解码）或 Err（发布失败），两者都不能让
                // `:q` 把这笔已提交编辑静默丢掉。
                self.dirty = true;
                self.sync_from_port();
                self.publish_snapshot(what);
            }
            Err(refusal) => self.log_line(format!("{what} 被拒绝：{refusal}")),
        }
    }

    /// 以撤销端口的当前版刷新本地镜像（提交/撤销/重做后必走）。
    fn sync_from_port(&mut self) {
        self.project = self.port.project();
        // 镜像换了 ⇒ 自动化缓存也要换一份；下一次 `tick` 用 `Some` 重新种子。
        self.automation_project_pending = true;
        self.refresh_track_ids();
        // 音符视图的对象若已不在池中（撤销删池条目等），关掉视图，
        // ⛔ 不留一个指向不存在片段的编辑面。
        if let Some(ListView::Notes { clip_id, .. }) = self.list_view
            && !self.project.clip_pool.contains_key(&clip_id)
        {
            self.list_view = None;
        }
        // 自动化视图同理：泳道所属音轨没了（撤销删轨），视图一并关掉。
        if let Some(ListView::Automation { track_id, .. }) = self.list_view
            && !self.project.tracks.contains_key(&track_id)
        {
            self.list_view = None;
        }
    }

    /// 把当前镜像增量发布到引擎（标记取端口显示态，与 GUI 同一口径）。
    fn publish_snapshot(&mut self, what: &str) {
        let mark = EditMark::from_display(&self.port.display());
        match self.host.publish_project(&self.project, mark) {
            Ok(PublishOutcome::Published(_)) => {
                // T-FIX-244：发布分支只管发布状态，不再负责置脏。
                // 置脏在 commit_edit / apply_history_outcome 里跟着"模型已变更"走；
                // 否则保存后一次延迟发布成功会把已清掉的 dirty 错误地置回。
                self.publish_deferred = false;
                self.log_line(format!("{what}（已发布到引擎）"));
            }
            // ⭐ P0-7：SFZ／音频解码转到后台线程，本跳不发布；置位延后标记，
            //    `tick` 每跳重试，就绪后自动完成发布（与 GUI 心跳同一口径）。
            Ok(PublishOutcome::Deferred) => {
                self.publish_deferred = true;
                self.log_line(format!("{what}（解码转后台，就绪后自动发布）"));
            }
            Err(error) => {
                self.publish_deferred = false;
                self.log_line(format!("{what} 发布失败：{error:?}"));
            }
        }
    }

    /// ⭐ P0-7 延后发布的每跳重试：标记从端口显示态现算（与 `publish_snapshot`
    /// 同一口径）；就绪 ⇒ 完成发布（T-FIX-244：只清延后标记，**不**标脏，
    /// 标脏已移到编辑落点）；失败 ⇒ 响亮记日志并停止重试
    /// （同一错误不再每 16 ms 刷一遍 —— 下一次编辑会重新触发发布）。
    fn retry_deferred_publish(&mut self) {
        if !self.publish_deferred {
            return;
        }
        let mark = EditMark::from_display(&self.port.display());
        match self.host.publish_project(&self.project, mark) {
            Ok(PublishOutcome::Published(_)) => {
                // T-FIX-244：延迟发布成功只清延后标记，绝不改 dirty。
                // dirty 由编辑本身负责；保存后这次成功不得把 dirty 置回 true。
                self.publish_deferred = false;
                self.log_line("后台解码就绪（已发布到引擎）".to_owned());
            }
            Ok(PublishOutcome::Deferred) => {}
            Err(error) => {
                self.publish_deferred = false;
                self.log_line(format!("后台解码后的发布失败：{error:?}"));
            }
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
                // T-FIX-244：撤销/重做同样改了端口模型 ⇒ 立即置脏。
                // 置脏只跟"模型是否变更"绑定，不在发布分支里做。
                self.dirty = true;
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
        if self
            .project
            .tracks
            .get(&track_id)
            .is_some_and(|track| track.clips.contains_key(&placement_id))
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
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
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
            Some(ListView::Automation { selected, .. }) => {
                (self.automation_points().len(), selected)
            }
            None => return,
        };
        let next = if len == 0 {
            0
        } else {
            usize::try_from((selected as i32 + delta).clamp(0, len as i32 - 1)).unwrap_or(0)
        };
        match &mut self.list_view {
            Some(ListView::Notes { selected, .. })
            | Some(ListView::Pool { selected })
            | Some(ListView::Automation { selected, .. }) => {
                *selected = next;
            }
            None => {}
        }
        self.automation_follow_selection();
    }

    /// 列表视图内跳到首/尾（Vi `gg`/`G` 按视图分流到这里）。
    fn view_select_edge(&mut self, last: bool) {
        let len = match self.list_view {
            Some(ListView::Notes { .. }) => self.view_notes().len(),
            Some(ListView::Pool { .. }) => self.pool_rows().len(),
            Some(ListView::Automation { .. }) => self.automation_points().len(),
            None => return,
        };
        let next = if last { len.saturating_sub(1) } else { 0 };
        match &mut self.list_view {
            Some(ListView::Notes { selected, .. })
            | Some(ListView::Pool { selected })
            | Some(ListView::Automation { selected, .. }) => {
                *selected = next;
            }
            None => {}
        }
        self.automation_follow_selection();
    }

    /// 自动化视图的选中点决定 `-`/`+` 的落点：选中那一行 ⇒ 小节光标跟到它的 tick。
    fn automation_follow_selection(&mut self) {
        let Some(ListView::Automation { selected, .. }) = self.list_view else {
            return;
        };
        let Some(point) = self.automation_points().get(selected).copied() else {
            return;
        };
        let bar_ticks = self.bar_ticks().max(1);
        self.cursor_bar = usize::try_from(point.tick / bar_ticks).unwrap_or(usize::MAX);
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

    /// 改选中音符的时值（`Shift+←/→`，`[UI-NOTE-005]` 项 3）。
    ///
    /// 换算与 batch 构造都走 GUI 的**同一对**纯函数：
    /// `yeban_app::host::resize_duration_delta_for`（一档 = 一个吸附网格，
    /// 与无修饰 `←/→` 的平移同一个常量）与 `resize_duration_ops_for`
    /// （`Op::SetNoteTiming`，保住音符身份；逐音符夹紧到 1 tick 下限）。
    /// 空批（没选区 / 已在下限 / Δ = 0）⇒ ⛔ 不写空 op、如实记日志。
    pub fn resize_selected_note_by_grid(&mut self, direction: i8) {
        if !matches!(self.list_view, Some(ListView::Notes { .. })) {
            self.log_line("改时值只在音符视图内接入（片段网格里按 n 打开）".to_owned());
            return;
        }
        let Some((_, _, note)) = self.selected_note() else {
            self.log_line("音符视图内无选中音符".to_owned());
            return;
        };
        let delta = yeban_app::host::resize_duration_delta_for(direction);
        let ops = yeban_app::host::resize_duration_ops_for(
            &self.project,
            &[note.id.to_canonical_string()],
            delta,
        );
        if ops.is_empty() {
            self.log_line("音符已在 1 tick 下限，无变化".to_owned());
            return;
        }
        self.commit_edit("tui: resize note duration", ops, "改音符时值");
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
        let total = self.total_bars();
        let clamped = bar.min(total.saturating_sub(1));
        let ticks = u64::try_from(clamped).unwrap_or(0) * self.bar_ticks();
        self.host
            .send_transport(&[TransportCommand::SeekTicks(ticks)]);
        self.cursor_bar = clamped;
        if clamped == bar {
            self.log_line(format!("定位到第 {} 小节", clamped + 1));
        } else {
            self.log_line(format!(
                "工程只有 {total} 小节，已定位到第 {} 小节",
                clamped + 1
            ));
        }
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
    // 主视图（Session / Arrangement）—— T-FIX-76 第 3 项
    // ------------------------------------------------------------------

    /// 切换主视图（GUI `F5`/`F6` 与 `ui/switch_main_view` 的 TUI 等价落点）。
    ///
    /// 枚举本身是 GUI 的 [`View`]（同一个类型、同一组白名单值），TUI 只是换渲染：
    /// `Arrangement` 画线性时间轴（片段网格），`Session` 画场景 × 轨道矩阵。
    /// 切换视图**不改工程、不提交任何 op**（视图态与 GUI 一样住在会话里）。
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            self.log_line(format!("已在{}视图", view_label(view)));
            return;
        }
        self.view = view;
        // 音符/池/自动化三种列表视图都是 Arrangement 的内容变体，切到 Session
        // 后没有落点：关掉（⛔ 不留一个看不见却还在吃键的编辑面）。
        if view == View::Session && self.list_view.is_some() {
            self.list_view = None;
            self.log_line("切到 Session 视图：已关闭片段网格的列表视图".to_owned());
        }
        let scenes = self.project.scenes.len();
        if view == View::Session && scenes == 0 {
            self.log_line("切到 Session 视图（本工程没有场景：矩阵为空）".to_owned());
        } else {
            self.log_line(format!("切到{}视图", view_label(view)));
        }
        self.scene_selected = self.scene_selected.min(scenes.saturating_sub(1));
    }

    /// 视图的中文名（状态行 / 回执用）。
    #[must_use]
    pub const fn view(&self) -> View {
        self.view
    }

    // ------------------------------------------------------------------
    // 轨道高度（每轨基准 + 全局乘子）—— T-FIX-76 第 2 项
    // ------------------------------------------------------------------
    //
    // 两个旋钮与一条公式全部来自 `yeban_app::bridge`（GUI 的同一份视图态）：
    // `effective = clamp(base × percent / 100, MIN_TRACK_HEIGHT_PX, MAX_TRACK_HEIGHT_PX)`
    // 只在 `effective_track_height_px` 里发生一次。TUI 额外做的唯一一件事是
    // **离散化**：把有效像素折成终端行数 `ceil(px / DEFAULT_TRACK_HEIGHT_PX)` ——
    // 它不夹紧、不改变有效值，因此不存在第二处边界。

    /// 全局高度乘子（百分比；100 = 不缩放）。
    #[must_use]
    pub fn height_percent(&self) -> u32 {
        self.height.percent()
    }

    /// 一条轨在终端里的行数（≥ 1）：有效像素 ÷ 默认行高，向上取整。
    #[must_use]
    pub fn track_rows(&self, track_id: &EntityId) -> u16 {
        let effective = self.track_height_px(track_id).1;
        u16::try_from(effective.div_ceil(bridge::DEFAULT_TRACK_HEIGHT_PX))
            .unwrap_or(1)
            .max(1)
    }

    /// 一条轨的 `(基准, 有效)` 像素读数（状态行与回执用；夹紧已在有效值里）。
    #[must_use]
    pub fn track_height_px(&self, track_id: &EntityId) -> (u32, u32) {
        let key = track_id.to_canonical_string();
        (self.height.base_px(&key), self.height.effective_px(&key))
    }

    /// 设全局高度乘子（`[`/`]` 与 `:height% <n>` 的落点）。
    pub fn set_height_percent(&mut self, percent: u32) {
        if !self.height.set_percent(percent) {
            self.log_line(format!("高度乘子已是 {percent}%，无变化"));
            return;
        }
        self.log_height();
    }

    /// 全局高度乘子按 `delta` 个百分点调整（下限 0：有效值仍由夹紧兜住）。
    pub fn nudge_height_percent(&mut self, delta: i32) {
        let next = i64::from(self.height.percent()) + i64::from(delta);
        let percent = u32::try_from(next.max(0)).unwrap_or(0);
        self.set_height_percent(percent);
    }

    /// 设选中轨的**基准**行高（像素；`0` = 取消覆盖，回到默认）。与
    /// `ui/set_track_height` 的 `heightPx` 同口径（⚠ 该方法的 `0` 同样表示取消覆盖）。
    pub fn set_track_height_px(&mut self, px: u32) {
        let Some(track_id) = self.selected_track_id() else {
            self.log_line("先选中一条轨道，再调轨道高度".to_owned());
            return;
        };
        let key = track_id.to_canonical_string();
        if !self.height.set_track_px(&key, px) {
            self.log_line(if px == 0 {
                "这一轨没有高度覆盖，基准已是默认值".to_owned()
            } else {
                format!("轨道高度基准已是 {px} px，无变化")
            });
            return;
        }
        self.log_height();
    }

    /// 选中轨基准行高按 `delta` 像素调整（`Alt+Shift+↑/↓`）。
    ///
    /// 向下减到 0 以下是"取消覆盖"（`TrackHeightLayout::set_track_px` 对 `0`
    /// 的既有语义），⛔ 不在这里另造一个"最小基准"。
    pub fn nudge_track_height_px(&mut self, delta: i32) {
        let Some(track_id) = self.selected_track_id() else {
            self.log_line("先选中一条轨道，再调轨道高度".to_owned());
            return;
        };
        let base = i64::from(self.track_height_px(&track_id).0) + i64::from(delta);
        let px = u32::try_from(base.max(0)).unwrap_or(0);
        self.set_track_height_px(px);
    }

    fn log_height(&mut self) {
        let mut line = format!("高度乘子 {}%", self.height.percent());
        if let Some(track_id) = self.selected_track_id() {
            let (base, effective) = self.track_height_px(&track_id);
            let rows = self.track_rows(&track_id);
            line.push_str(&format!(
                "；选中轨基准 {base} px → 有效 {effective} px（{rows} 行）"
            ));
        }
        self.log_line(line);
    }

    // ------------------------------------------------------------------
    // 移轨（显示顺序绝对位置）—— T-FIX-76 第 1 项
    // ------------------------------------------------------------------

    /// 把选中轨在**显示顺序**里移动 `delta` 格（`J`/`K`，计数前缀 = 格数）。
    ///
    /// 与 `yeban_move_track` 同一组拒绝：主总线不可移（不在 `display_track_ids`
    /// 里 ⇒ 同一条拒绝理由）、越界不可移；落点都是同一条 [`Op::MoveTrack`]
    /// （绝对下标形态，`0` 起），⛔ 不另造相对下标的 op。
    pub fn move_selected_track(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let Some(track_id) = self.selected_track_id() else {
            self.log_line("先选中一条轨道，再移轨".to_owned());
            return;
        };
        let ids = self.project.display_track_ids();
        let Some(old_index) = ids.iter().position(|id| *id == track_id) else {
            self.log_line("选中轨是工程主总线：主总线不参与显示顺序，不能移动".to_owned());
            return;
        };
        let target = i64::try_from(old_index).unwrap_or(0) + i64::from(delta);
        if target < 0 || target >= i64::try_from(ids.len()).unwrap_or(0) {
            let edge = if delta > 0 { "末位" } else { "首位" };
            self.log_line(format!(
                "已在{edge}：显示顺序只有 {} 条轨，没有可移动的位置",
                ids.len()
            ));
            return;
        }
        let new_index = usize::try_from(target).unwrap_or(0);
        self.apply_move_track(track_id, old_index, new_index);
    }

    /// 把选中轨移到**绝对位置** `new_index`（`0` 起，与 `yeban_move_track` 的
    /// `newIndex` 同口径；`:move <index>` 的落点）。
    ///
    /// 预校验逐条镜像 MCP 的 `plan_move_track`：不存在 → 主总线拒绝 →
    /// 越界拒绝（报出轨道条数）→ 原地拒绝；通过后才提交一条 op。
    pub fn move_selected_track_to(&mut self, new_index: usize) {
        let Some(track_id) = self.selected_track_id() else {
            self.log_line("先选中一条轨道，再移轨".to_owned());
            return;
        };
        let ids = self.project.display_track_ids();
        let Some(old_index) = ids.iter().position(|id| *id == track_id) else {
            self.log_line("选中轨是工程主总线：主总线不参与显示顺序，不能移动".to_owned());
            return;
        };
        if new_index >= ids.len() {
            self.log_line(format!(
                "目标位置 {new_index} 越界：显示顺序只有 {} 条轨（0 起）",
                ids.len()
            ));
            return;
        }
        if new_index == old_index {
            self.log_line(format!("这一轨已在位置 {new_index}，没有可移动的位置"));
            return;
        }
        self.apply_move_track(track_id, old_index, new_index);
    }

    fn apply_move_track(&mut self, track_id: EntityId, old_index: usize, new_index: usize) {
        let name = self
            .project
            .tracks
            .get(&track_id)
            .map(|track| track.name.clone())
            .unwrap_or_default();
        self.commit_edit(
            "tui: move track display order",
            vec![Op::MoveTrack {
                track_id,
                old_index,
                new_index,
            }],
            &format!("「{name}」移到第 {} 位", new_index + 1),
        );
        // 选中跟随被移动的轨（不是跟随下标：下标在移动后指向别的轨）。
        if let Some(index) = self.track_ids.iter().position(|id| *id == track_id) {
            self.selected = index;
        }
    }

    // ------------------------------------------------------------------
    // 自动化编辑（`yeban_edit_automation` 的 TUI 入口）—— T-FIX-76 第 4 项
    // ------------------------------------------------------------------

    /// 打开选中轨的自动化泳道（`p` = 音量、`P` = 声相、`:auto <volume|pan>`）。
    pub fn open_automation_view(&mut self, kind: AutomationKind) {
        let Some(track_id) = self.selected_track_id() else {
            self.log_line("先选中一条轨道，再开自动化视图".to_owned());
            return;
        };
        if !self.project.tracks.contains_key(&track_id) {
            self.log_line("选中的轨已不在工程里：先重新选一条轨".to_owned());
            return;
        }
        self.open_automation_target(kind.target(track_id));
    }

    /// 打开任意目标（六类同一入口）：**先对账、再开视图**。
    ///
    /// 对账走 [`AutomationTarget::validate_against`] —— 设备槽 / 参数序 / 宏号越界、
    /// 路由边不存在、CC 控制器越界都在这里被拒，错误文本是模型给的那条具体错误。
    /// 被拒时 ⛔ **不开视图、不建泳道**（默认不能静默）；成功时视图只是"打开目标"，
    /// 泳道本身由首个 `Op::SetAutomationPoint` 落地（模型语义：空泳道不落盘）。
    pub fn open_automation_target(&mut self, target: AutomationTarget) {
        let track_id = target.track_id();
        if let Err(error) = target.validate_against(&self.project) {
            self.log_line(format!(
                "目标 {} 在工程里不存在：{error}（未打开泳道）",
                yeban_app::automation::target_key(&target)
            ));
            return;
        }
        self.list_view = Some(ListView::Automation {
            track_id,
            target,
            selected: 0,
        });
        self.focus = Pane::Grid;
        let domain = self.automation_domain().map_or_else(
            || "值域未知".to_owned(),
            |(min, max)| format!("值域 [{min}, {max}]"),
        );
        let kind = kind_of(target);
        self.log_line(format!(
            "自动化视图：{} · {}（{domain}）：a 在光标处设点 · -/+ 改值 · Delete 删点 · Esc 返回",
            kind.label(),
            self.target_display_name(&target)
        ));
    }

    /// 目标的人读名（**与 GUI 同一命名口径**：直接调 `yeban_app::automation::target_label`，
    /// ⛔ TUI 里不再写一份"设备参数叫什么"的规则）。
    #[must_use]
    pub fn target_display_name(&self, target: &AutomationTarget) -> String {
        self.project.track(&target.track_id()).map_or_else(
            |_| format!("{}（轨道不在工程里）", kind_of(*target).label()),
            |track| yeban_app::automation::target_label(track, target),
        )
    }

    /// 当前泳道的人读名（视图未开时 `"自动化"`）。
    #[must_use]
    pub fn automation_lane_name(&self) -> String {
        self.automation_target().map_or_else(
            || "自动化".to_owned(),
            |target| self.target_display_name(&target),
        )
    }

    /// 选中轨上**已有**的自动化泳道（`(目标, 点数)`，按 `AutomationTarget::Ord` 升序 ——
    /// `BTreeMap` 迭代序，与 GUI 投影同一序）。
    #[must_use]
    pub fn automation_lanes(&self) -> Vec<(AutomationTarget, usize)> {
        let Some(track_id) = self.selected_track_id() else {
            return Vec::new();
        };
        self.project.track(&track_id).map_or_else(
            |_| Vec::new(),
            |track| {
                track
                    .automation_lanes
                    .iter()
                    .map(|(target, lane)| (*target, lane.points.len()))
                    .collect()
            },
        )
    }

    /// 泳道清单的可读行（`:auto` 无参时逐行写出）：序号 · 目标名 · 单位 · 点数。
    #[must_use]
    pub fn automation_lane_lines(&self) -> Vec<String> {
        self.automation_lanes()
            .iter()
            .enumerate()
            .map(|(index, (target, points))| {
                format!(
                    "{index}. {} · {} · {points} 个点",
                    self.target_display_name(target),
                    target.nominal_unit().symbol()
                )
            })
            .collect()
    }

    /// 打开清单里的第 `index` 条泳道（`:auto <序号>`）。
    pub fn open_automation_lane(&mut self, index: usize) {
        let lanes = self.automation_lanes();
        match lanes.get(index) {
            Some((target, _)) => self.open_automation_target(*target),
            None => self.log_line(format!(
                "没有第 {index} 条泳道：先 `:auto` 看清单（共 {} 条）",
                lanes.len()
            )),
        }
    }

    /// `:auto` 的完整语法（批 5：六类目标同一入口）。
    ///
    /// - `:auto`：列出选中轨**已有**的泳道（序号 · 目标名 · 单位 · 点数）；
    /// - `:auto <序号>`：打开清单里的那一条；
    /// - `:auto volume` / `:auto pan`：既有的两条快捷泳道；
    /// - `:auto send <路由边ULID>` / `:auto device <设备槽> <参数序>` /
    ///   `:auto macro <宏号>` / `:auto cc <0-127>`：打开四类目标泳道。
    ///
    /// 选择面沿用 TUI 既有惯例（命令 + 序号清单），⛔ 不新造控件框架；
    /// 目标是否存在由 [`App::open_automation_target`] 的对账决定。
    fn command_automation(&mut self, rest: &str) {
        const USAGE: &str = "用法：:auto（列现有泳道）｜:auto <序号>｜:auto volume|pan｜:auto send <路由边ULID>｜:auto device <设备槽> <参数序>｜:auto macro <宏号>｜:auto cc <0-127>";
        if self.selected_track_id().is_none() {
            self.log_line("先选中一条轨道，再用 :auto".to_owned());
            return;
        }
        let words: Vec<&str> = rest.split_whitespace().collect();
        match words.as_slice() {
            [] => {
                let lines = self.automation_lane_lines();
                if lines.is_empty() {
                    self.log_line(
                        "选中轨还没有自动化泳道：:auto volume|pan|send|device|macro|cc 开一条"
                            .to_owned(),
                    );
                } else {
                    for line in lines {
                        self.log_line(line);
                    }
                }
                self.log_line(USAGE.to_owned());
            }
            ["volume"] => self.open_automation_view(AutomationKind::Volume),
            ["pan"] => self.open_automation_view(AutomationKind::Pan),
            ["send", edge] => match edge.parse::<EntityId>() {
                Ok(edge_id) => self.open_automation_view(AutomationKind::Send { edge_id }),
                Err(_) => self.log_line(format!("路由边 `{edge}` 不是合法的 EntityId")),
            },
            ["device", slot, param] => match (slot.parse::<usize>(), param.parse::<usize>()) {
                (Ok(slot_index), Ok(param_index)) => {
                    self.open_automation_view(AutomationKind::Device {
                        slot_index,
                        param_index,
                    })
                }
                _ => self.log_line(format!("设备槽 `{slot}` / 参数序 `{param}` 必须是非负整数")),
            },
            ["macro", index] => match index.parse::<usize>() {
                Ok(macro_index) => self.open_automation_view(AutomationKind::Macro { macro_index }),
                Err(_) => self.log_line(format!("宏号 `{index}` 必须是非负整数")),
            },
            // CC 先按 `u8` 解析（128..=255 能被 `u8` 收下，再由模型的对账拒掉）——
            // ⛔ 不在 TUI 里另定一条 0..=127 的规则，域的唯一出处是 `validate_against`。
            ["cc", controller] => match controller.parse::<u8>() {
                Ok(controller) => self.open_automation_view(AutomationKind::MidiCc { controller }),
                Err(_) => self.log_line(format!("控制器号 `{controller}` 必须是 0..=255 的整数")),
            },
            [index] => match index.parse::<usize>() {
                Ok(index) => self.open_automation_lane(index),
                Err(_) => self.log_line(USAGE.to_owned()),
            },
            _ => self.log_line(USAGE.to_owned()),
        }
    }

    /// 自动化视图的泳道目标（视图未开时 `None`）。
    #[must_use]
    pub fn automation_target(&self) -> Option<AutomationTarget> {
        match self.list_view {
            Some(ListView::Automation { target, .. }) => Some(target),
            _ => None,
        }
    }

    /// 当前泳道在**选中点精确 tick** 处的三态读数（走模型唯一求值入口）。
    ///
    /// 没有选中的点（空泳道）时回退到小节光标 tick，保持“开视图即可设点”的
    /// 原有行为；一旦有选中点，读数必须跟着点的精确 tick，不能跟着小节光标走
    /// —— 小节光标只到小节边界，`tick=1920` 的点会读成 `tick=0`。
    #[must_use]
    pub fn automation_readout(&self) -> Option<AutomationReadout> {
        self.automation_readout_at(self.automation_edit_tick())
    }

    /// 在指定 tick 处构造三态读数（只读；不发编辑）。
    fn automation_readout_at(&self, tick: u64) -> Option<AutomationReadout> {
        let target = self.automation_target()?;
        let automation_value = self.project.automation_value_at(&target, tick).ok()?;
        let static_value = target.static_value(&self.project).ok()?;
        let (effective_value, source) = match automation_value {
            Some(value) => (value, "automation"),
            None => (static_value, "static"),
        };
        Some(AutomationReadout {
            tick,
            automation_value,
            static_value,
            effective_value,
            source,
        })
    }

    /// 自动化视图当前选中的采样点（按 `(tick, 点身份)` 序；视图未开或空泳道为 `None`）。
    fn selected_automation_point(&self) -> Option<AutomationPoint> {
        let Some(ListView::Automation { selected, .. }) = self.list_view else {
            return None;
        };
        self.automation_points().get(selected).copied()
    }

    /// 自动化编辑/删除使用的精确 tick：有选中点读 `point.tick`，否则读小节光标。
    fn automation_edit_tick(&self) -> u64 {
        self.selected_automation_point()
            .map_or_else(|| self.automation_cursor_tick(), |point| point.tick)
    }

    /// 光标所在的 tick（小节光标 × 每小节 tick）。
    #[must_use]
    pub fn automation_cursor_tick(&self) -> u64 {
        u64::try_from(self.cursor_bar)
            .unwrap_or(u64::MAX)
            .saturating_mul(self.bar_ticks())
    }

    /// 当前泳道的采样点（按 `(tick, 点身份)` 升序；视图未开时为空）。
    #[must_use]
    pub fn automation_points(&self) -> Vec<AutomationPoint> {
        let Some(target) = self.automation_target() else {
            return Vec::new();
        };
        self.project
            .automation_lane(&target)
            .map(yeban_model::AutomationLane::points_in_tick_order)
            .unwrap_or_default()
    }

    /// 当前泳道的有效值域 `(min, max)`（泳道覆盖优先，否则目标固有值域）。
    #[must_use]
    pub fn automation_domain(&self) -> Option<(f32, f32)> {
        let target = self.automation_target()?;
        let domain = self
            .project
            .automation_lane(&target)
            .and_then(|lane| lane.effective_domain())
            .or_else(|| target.nominal_domain())?;
        Some((domain.min(), domain.max()))
    }

    /// 在**选中点的精确 tick** 写一个自动化点（没有选中点时退回小节光标；
    /// 一条 `Op::SetAutomationPoint`，可撤销）。
    ///
    /// 与 `yeban_edit_automation` 同一条值域闸门：非有限或越出有效值域 ⇒
    /// **响亮拒绝**（⛔ 不夹紧 —— 夹紧会让"用户写的值"与"落盘的值"不一致）。
    /// 同 tick 的点按**既有身份**更新（与 MCP 的"重复写入是更新，不是堆积"同一承诺）。
    pub fn set_automation_value(&mut self, value: f32) {
        let tick = self.automation_edit_tick();
        self.set_automation_value_at(tick, value);
    }

    /// [`Self::set_automation_value`] 的显式 tick 版本；`add_automation_point_at_cursor`
    /// 必须落在小节光标，而不是被选中的别的点。
    fn set_automation_value_at(&mut self, tick: u64, value: f32) {
        let Some(target) = self.automation_target() else {
            self.log_line("先开自动化视图（p 音量 / P 声相 / :auto ...），再设点".to_owned());
            return;
        };
        // 视图开着时设备槽 / 宏 / 路由边也可能被撤销掉 ⇒ 每次写入前再对一次账，
        // ⛔ 不往一个不存在的目标上写点（模型的存在性检查只在求值入口，写点不查）。
        if let Err(error) = target.validate_against(&self.project) {
            self.log_line(format!(
                "目标 {} 在工程里不存在：{error}（拒绝写点）",
                yeban_app::automation::target_key(&target)
            ));
            return;
        }
        if !value.is_finite() {
            self.log_line(format!("值 {value} 不是有限数：拒绝写点"));
            return;
        }
        if let Some((min, max)) = self.automation_domain()
            && (value < min || value > max)
        {
            self.log_line(format!(
                "值 {value} 越出本泳道的有效取值域 [{min}, {max}]：拒绝写点"
            ));
            return;
        }
        let existing = self.project.automation_lane(&target).and_then(|lane| {
            lane.points
                .values()
                .find(|point| point.tick == tick)
                .copied()
        });
        let point_id = existing.map_or_else(EntityId::new, |point| point.id);
        let curve = existing.map_or(CurveType::Linear, |point| point.curve);
        let new_point = AutomationPoint {
            id: point_id,
            tick,
            value,
            curve,
        };
        let what = format!(
            "自动化点 {} @ {tick} = {value:.2} {}",
            if existing.is_some() {
                "更新"
            } else {
                "新增"
            },
            self.automation_unit_symbol()
        );
        self.commit_edit(
            "tui: set automation point",
            vec![Op::SetAutomationPoint {
                target,
                point_id,
                old_point: existing,
                new_point,
            }],
            &what,
        );
        self.select_automation_point_at(tick);
    }

    /// 光标 tick 处的自动化值按 `steps` 个步长微调（音量 0.5 dB / 声相 0.05）。
    pub fn nudge_automation_value(&mut self, steps: i32) {
        let Some(readout) = self.automation_readout() else {
            self.log_line("先开自动化视图（p 音量 / P 声相），再改值".to_owned());
            return;
        };
        let Some(kind) = self.automation_kind() else {
            return;
        };
        let next = readout.effective_value + kind.step(self.automation_domain()) * (steps as f32);
        self.set_automation_value(next);
    }

    /// `Ctrl+Alt+A` 的落点：已开自动化视图 ⇒ 关掉；否则开**音量**泳道。
    pub fn toggle_automation_view(&mut self) {
        if matches!(self.list_view, Some(ListView::Automation { .. })) {
            self.list_view = None;
            self.log_line("关闭自动化视图".to_owned());
        } else {
            self.open_automation_view(AutomationKind::Volume);
        }
    }

    /// 当前自动化视图的泳道种类（视图未开或非音量/声相时 `None`）。
    #[must_use]
    pub fn automation_kind(&self) -> Option<AutomationKind> {
        self.automation_target().map(kind_of)
    }

    /// 删除当前选中的自动化点（按选中点的**精确 `point.tick`**，不再按小节截断；
    /// `Op::RemoveAutomationPoint`，可撤销）。
    pub fn remove_automation_point_at_cursor(&mut self) {
        let Some(target) = self.automation_target() else {
            self.log_line("先开自动化视图（p 音量 / P 声相），再删点".to_owned());
            return;
        };
        let Some(point) = self.selected_automation_point() else {
            self.log_line("当前没有选中的自动化点：先用 j/k 在泳道里选一行".to_owned());
            return;
        };
        let tick = point.tick;
        self.commit_edit(
            "tui: remove automation point",
            vec![Op::RemoveAutomationPoint {
                target,
                point_id: point.id,
                previous_point: point,
            }],
            &format!("删除自动化点 @ {tick}"),
        );
    }

    /// 在小节光标 tick 处按**当前生效值**落一个点（`a`）：把"此刻听起来的样子"钉下来。
    ///
    /// 注意：这里读的是小节光标处的值，**不是**选中点处的值；`a` 的语义是
    /// “在光标处设点”，不能因为别的点被选中就把点写到那个点上。
    pub fn add_automation_point_at_cursor(&mut self) {
        let tick = self.automation_cursor_tick();
        let Some(readout) = self.automation_readout_at(tick) else {
            self.log_line("先开自动化视图（p 音量 / P 声相），再设点".to_owned());
            return;
        };
        self.set_automation_value_at(tick, readout.effective_value);
    }

    fn automation_unit_symbol(&self) -> &'static str {
        self.automation_target()
            .map_or("", |target| target.nominal_unit().symbol())
    }

    fn select_automation_point_at(&mut self, tick: u64) {
        let index = self
            .automation_points()
            .iter()
            .position(|point| point.tick == tick);
        if let (Some(index), Some(ListView::Automation { selected, .. })) =
            (index, &mut self.list_view)
        {
            *selected = index;
        }
    }

    // 引擎状态 / 通道电平 / 管理动作（T-FIX-81）
    // ------------------------------------------------------------------

    /// 一条轨的最新峰值（线性幅度；没有读数 = `0.0`）。
    #[must_use]
    pub fn track_peak(&self, track_id: &EntityId) -> f32 {
        self.track_peaks.get(track_id).copied().unwrap_or(0.0)
    }

    /// 判据专用：推 `quanta` 个量子再抽干电平（与主循环 `tick` 同一条抽干路径）。
    ///
    /// `#[doc(hidden)]`：不是给界面用的入口 —— 外部判据需要复现"引擎出声 ⇒ 表动"
    /// 这一条链，而 `tick` 的墙钟泵送在测试里要真等时间。⛔ 不要在生产路径调用。
    #[doc(hidden)]
    pub fn drive_meters_for_test(&mut self, quanta: u64) {
        self.host.drive_audio(quanta);
        self.tick();
    }

    /// 判据专用：快照自己的 `(sample_rate, revision)` 读数（`None` = 没有引擎）。
    ///
    /// `#[doc(hidden)]`：只为外部判据核对"引擎状态面读的是快照自己的数"，
    /// ⛔ 界面不许绕过 [`App::engine_status`] 另开一条读数。
    #[doc(hidden)]
    pub fn snapshot_readings(&self) -> (Option<u32>, Option<u64>) {
        self.host
            .current_snapshot()
            .map_or((None, None), |snapshot| {
                (Some(snapshot.sample_rate()), Some(snapshot.revision()))
            })
    }

    /// 引擎状态读数（`yeban_query_engine_state` 的 TUI 等价面）。
    #[must_use]
    pub fn engine_status(&self) -> EngineStatus {
        let snapshot = self.host.current_snapshot();
        let reading = self.transport();
        let device_active = self.host.device_active();
        let device = self.device_status();
        let conversion = match (device.source_rate, device.device_sample_rate) {
            (Some(source), Some(target)) if source != target => {
                format!("采样率转换: 工程 {source} Hz → 设备 {target} Hz")
            }
            (Some(source), Some(_)) => {
                format!("无采样率转换（工程 {source} Hz 与设备率一致）")
            }
            _ => "未开设备腿（TUI 走墙钟泵送）⇒ 无实时采样率转换".to_owned(),
        };
        EngineStatus {
            project_sample_rate: device.project_sample_rate,
            engine_sample_rate: snapshot.as_ref().map(|snapshot| snapshot.sample_rate()),
            block_frames: snapshot
                .as_ref()
                .map(|snapshot| u32::try_from(snapshot.block_frames()).unwrap_or(u32::MAX)),
            channels: snapshot.as_ref().map(|snapshot| snapshot.channels()),
            device_name: device.device_name.clone(),
            device_sample_rate: device.device_sample_rate,
            device_active,
            conversion,
            transport_state: reading.state,
            position_ticks: reading.position_ticks,
        }
    }

    /// 设备状态行读数（T-FIX-82④）：设备名 / 设备采样率 / 工程采样率 / 转换状态。
    ///
    /// 设备侧两个数来自 `EngineHost::device_rates()`（协商结果）；工程率来自
    /// `project.audio_config`。⛔ 不从工程推算设备率 —— 没开设备腿就是 `None`。
    #[must_use]
    pub fn device_status(&self) -> DeviceStatus {
        let rates = self.host.device_rates();
        DeviceStatus {
            device_name: self.host.device_name().map(str::to_owned),
            source_rate: rates.map(|(source, _)| source),
            device_sample_rate: rates.map(|(_, device)| device),
            project_sample_rate: sample_rate_hz(self.project.audio_config.sample_rate),
        }
    }

    /// 设备状态行文本（底栏 `engine_line` 的同一份读数；判据与界面共用）。
    #[must_use]
    pub fn device_status_line(&self) -> String {
        self.device_status().line()
    }

    /// 工程信息面读数（`:info` 的 TUI 等价面）：契约 + 960 PPQ 时钟 + 身份 + 规模。
    ///
    /// 版本字段直接取 `YebanProjectV1` 自己的 `schema_version` / `min_reader_version` /
    /// `writer_version`，可读性用**同一个** `check_readable`（打开工程时的同一道门，
    /// ⛔ 不另写一份版本比较）；tick 换算复用时间码/吸附用的 `ticks_per_beat`。
    #[must_use]
    pub fn project_info(&self) -> ProjectInfo {
        let selected_track = self.selected_track_id().map(|id| {
            let name = self
                .project
                .tracks
                .get(&id)
                .map(|track| track.name.clone())
                .unwrap_or_default();
            (id.to_canonical_string(), name)
        });
        ProjectInfo {
            title: self.project.title.clone(),
            id: self.project.id.to_canonical_string(),
            schema_version: self.project.schema_version,
            min_reader_version: self.project.min_reader_version,
            writer_version: self.project.writer_version.clone(),
            readable: self
                .project
                .check_readable()
                .map_err(|error| error.to_string()),
            ppq: yeban_model::ids::PPQ,
            ticks_per_beat: self.ticks_per_beat(),
            bar_ticks: self.bar_ticks(),
            numerator: self.project.time_signature.numerator,
            denominator: self.project.time_signature.denominator,
            total_bars: self.total_bars(),
            length_ticks: self.project_len_ticks(),
            tracks: self.project.tracks.len(),
            clips: self.project.clip_pool.len(),
            source: self
                .project_path
                .as_ref()
                .map(|path| path.display().to_string()),
            selected_track,
            cursor_bar: self.cursor_bar,
        }
    }

    /// 工程信息面的四行文本（`:info` / `:clock` 的落点）。
    #[must_use]
    pub fn project_info_lines(&self) -> Vec<String> {
        self.project_info().lines()
    }

    /// 当前界面的语义清单（T-FIX-86③）：三个窗格 + 轨道行 + 检查器文本，
    /// 角色全部取自 GUI 的 `ElementKind::accessible_role`，序按显示顺序。
    ///
    /// 终端拿不到 OS 无障碍树（取舍写在 README），这份清单是**可核的语义面**：
    /// `:a11y` 逐行导出 `role / id / index / label`，判据按它断言"标签与界面上
    /// 看到的文本同源、序连续"。
    #[must_use]
    pub fn semantic_rows(&self) -> Vec<SemanticRow> {
        use yeban_app::elements::ElementKind;
        let mut rows = Vec::with_capacity(self.track_ids.len() + 8);
        rows.push(SemanticRow {
            id: "tui-main".to_owned(),
            role: ElementKind::Main.accessible_role(),
            label: format!("{} 主工作区", self.project.title),
            index: 0,
        });
        rows.push(SemanticRow {
            id: "tui-tracks".to_owned(),
            role: ElementKind::List.accessible_role(),
            label: format!("轨道栏（{} 条）", self.track_ids.len()),
            index: 0,
        });
        for (index, id) in self.track_ids.iter().enumerate() {
            let name = self
                .project
                .tracks
                .get(id)
                .map_or_else(String::new, |track| track.name.clone());
            let selected = if index == self.selected {
                "（选中）"
            } else {
                ""
            };
            rows.push(SemanticRow {
                id: format!("tui-track-{index}"),
                role: ElementKind::ListItem.accessible_role(),
                label: format!("轨道 {index}{selected} {name}"),
                index,
            });
        }
        rows.push(SemanticRow {
            id: "tui-grid".to_owned(),
            role: ElementKind::Region.accessible_role(),
            label: if self.list_view.is_some() {
                "片段网格（列表视图）".to_owned()
            } else {
                "片段网格".to_owned()
            },
            index: 0,
        });
        rows.push(SemanticRow {
            id: "tui-inspector".to_owned(),
            role: ElementKind::GroupBox.accessible_role(),
            label: "检查器".to_owned(),
            index: 0,
        });
        let track = self
            .selected_track_id()
            .and_then(|id| self.project.tracks.get(&id));
        let inspector_labels: Vec<String> = match track {
            Some(track) => vec![
                format!("名称 {}", track.name),
                format!("类型 {}", track_kind_label(track.kind)),
                format!("音量 {:+.1} dB", track.volume_db),
                format!("声相 {:+.1}", track.pan),
                format!("身份 {}", track.id.to_canonical_string()),
            ],
            None => vec!["未选中轨".to_owned()],
        };
        for (index, label) in inspector_labels.into_iter().enumerate() {
            rows.push(SemanticRow {
                id: format!("tui-inspector-text-{index}"),
                role: ElementKind::Text.accessible_role(),
                label,
                index,
            });
        }
        rows.push(SemanticRow {
            id: "tui-status".to_owned(),
            role: ElementKind::ContentInfo.accessible_role(),
            label: self.log.last().cloned().unwrap_or_default(),
            index: 0,
        });
        rows
    }

    /// 语义清单的文本行（`:a11y` 的落点；判据与界面共用）。
    #[must_use]
    pub fn semantic_lines(&self) -> Vec<String> {
        self.semantic_rows()
            .iter()
            .map(|row| {
                format!(
                    "role={} id={} index={} label={}",
                    row.role, row.id, row.index, row.label
                )
            })
            .collect()
    }

    /// 置位 / 清零输入法合成态（T-FIX-86④）。
    ///
    /// 终端协议**不上报**合成态，所以 TUI 不能自己"检测"到它；这个入口留给
    /// 能检测的宿主（Windows 控制台桥、将来接 IME 的包装层）以及判据。
    /// 合成中：除 `F5`/`F6` 外的按键全部吞掉（与 GUI 解析器第 1/2 步同一规则），
    /// ⛔ 绝不让候选词按键冒泡成走带/编辑热键。
    pub fn set_ime_composing(&mut self, composing: bool) {
        if composing {
            self.input_ctx.begin_composition();
        } else {
            self.input_ctx.end_composition();
        }
    }

    /// 当前是否处于合成态（与 `InputContext` 是同一份状态）。
    #[must_use]
    pub fn ime_composing(&self) -> bool {
        self.input_ctx.is_composing()
    }

    /// 强制保存（GUI `ui/force_save` 的 TUI 等价）：**真的写盘**，不因"看起来没变"而跳过。
    ///
    /// 与 `ui/force_save` 同一条硬要求：必须有一个**真的路径**，没有就如实报错
    /// （⛔ 不假成功、不偷偷写到一个编出来的文件名）。
    ///
    /// # Errors
    /// 无路径或写盘失败时返回错误文本。
    pub fn force_save(&mut self) -> Result<String, String> {
        let Some(path) = self.project_path.clone() else {
            return Err(
                "强制保存失败：尚无文件路径（先 :w <path> 指定，或 :open <path> 打开）".to_owned(),
            );
        };
        let done = self.save_to(&path)?;
        Ok(format!("{done}（强制保存）"))
    }

    /// 重载音频引擎（GUI `ui/reload_engine` 的 TUI 等价）：按当前文档重建一代引擎。
    ///
    /// 与 GUI 同一个落点（`EngineHost::reload`），失败**原文**记日志（`Debug` 形态，
    /// 与 TUI 其它引擎错误同一口径）；成功后采纳新的电平消费端并停住走带
    /// （与打开工程时"加载即停住"同一条纪律）。
    pub fn reload_engine(&mut self) {
        match self.host.reload(&self.project, 1) {
            Ok(rebuild) => {
                self.collector = Some(rebuild.collector);
                self.host.stop();
                // 新的一代已经反映当前文档 ⇒ 重钉"已施加"标记（与 GUI 同一口径）。
                self.host
                    .mark_applied(EditMark::from_display(&self.port.display()));
                self.log_line(format!(
                    "重载音频引擎：第 {} 代 · revision {} · {} 轨 · 发布 {}/{} 帧 (量子 {}) · 容量丢弃 {}",
                    rebuild.generation,
                    rebuild.revision,
                    rebuild.tracks,
                    rebuild.meter_bulk_publishes,
                    rebuild.meter_frames,
                    rebuild.quanta,
                    rebuild.meter_capacity_drops,
                ));
            }
            Err(error) => self.log_line(format!("重载音频引擎失败：{error:?}")),
        }
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

    /// 保存到指定路径。
    ///
    /// 与 MCP/GUI 的 `yeban_save_project` 共用 `embed_samples::collect`：先把当前
    /// 音符实际触发的新样本收进归档、刷新 `embedded_samples` 表与资产池，再走
    /// **唯一**的归档写入口 [`yeban_app::save::save_archive_file`]。
    ///
    /// # Errors
    /// 内嵌收集失败（SFZ 不可读 / 非法 UTF-8 / 解析失败）或写盘失败时返回错误文本。
    pub fn save_to(&mut self, path: &Path) -> Result<String, String> {
        let source_path = self.project_path.as_deref().unwrap_or(path);
        let mut project = self.port.project();
        // 端口工程只持有用户编辑；内嵌表由保存侧维护。若上一次保存已把新样本
        // 内嵌，仍要从归档表里复用——否则盘上样本消失后重存会把它剪掉。
        if let Some(archive) = &self.archive {
            project.embedded_samples = archive.project.embedded_samples.clone();
        }
        let (archive_with_embeds, missing_samples) =
            prepare_archive_for_save(project, source_path, self.archive.as_ref())?;
        let mut archive = archive_with_embeds;
        archive.history_dag = yeban_model::commit::encode_history_dag(&self.port.graph());
        yeban_app::save::save_archive_file(&archive, path)
            .map_err(|error| format!("保存失败：{error}"))?;
        // 落盘成功后再换内存里的归档：失败时保留旧池，下一次保存仍可复用。
        self.project.embedded_samples = archive.project.embedded_samples.clone();
        // 引擎宿主的字节来源也换成刚写出的资产池：保存后再重载引擎时，
        // 新内嵌的样本不依赖原盘上文件仍然存在。
        let asset_bytes: BTreeMap<AssetHash, Vec<u8>> = archive.assets.iter().cloned().collect();
        self.host
            .set_audio_asset_source(Box::new(move |hash: &AssetHash| {
                asset_bytes.get(hash).cloned()
            }));
        self.archive = Some(archive);
        self.project_path = Some(path.to_path_buf());
        if let Some(dir) = path.parent() {
            self.host.set_project_dir(Some(dir.to_path_buf()));
        }
        self.dirty = false;
        let missing = if missing_samples == 0 {
            String::new()
        } else {
            format!("；{missing_samples} 个样本缺失，未内嵌")
        };
        Ok(format!("已保存 {}{missing}", path.display()))
    }

    /// 离线导出 WAV（生产渲染路径：引擎乐器，非最小音源）。
    ///
    /// # Errors
    /// 渲染或写盘失败时返回错误文本。
    pub fn export_wav(&mut self, path: &Path) -> Result<ExportReceipt, String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.is_dir()
        {
            return Err(format!(
                "写 WAV 失败：目录不存在 `{}`（先创建目录，或换一个已存在的路径）",
                parent.display()
            ));
        }
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

    // ───────────── T-FIX-82 批 3：鼓机 / 分轨 / 诊断包 / 设备腿 ─────────────

    /// 鼓机生成（GUI「鼓机生成 (Drums)...」与 `yeban_generate_drum_pattern` 的 TUI 等价面）。
    ///
    /// 配方与 GUI / MCP **同一份**：风格预置走 `yeban_theory::factory_drum_style`，
    /// 旋钮范围与缺省同一个口径（复杂度 0..=100 缺省 50、强度 0..=100 缺省 80、
    /// 摇摆 500..=1000 缺省取风格定值、种子缺省 0、小节 1..=16 缺省 4、fill 缺省开），
    /// 音符由 `yeban_theory::generate_drum_pattern` 生成，`Op` 批次由
    /// `yeban_app::bridge::drum_gen_ops` 构造（目标轨是 MIDI 展示轨 ⇒ 落该轨并沿用
    /// 其鼓组，否则新建鼓轨并按风格族装默认鼓组）。落点 tick 0（与 GUI 一致）。
    ///
    /// # Errors
    ///
    /// 风格不存在 / 旋钮越界 / 目标不可写 / 生成为空 / 提交被拒时返回原因文本。
    pub fn generate_drum_pattern(&mut self, request: &DrumRequest) -> Result<String, String> {
        request.validate()?;
        let style = request.style.as_str();
        let preset = yeban_theory::factory_drum_style(style)
            .map_err(|error| format!("风格预置 `{style}` 解析失败: {error}"))?
            .ok_or_else(|| {
                format!(
                    "没有名为 `{style}` 的鼓机风格预置（可用：{}）",
                    yeban_theory::factory_drum_style_ids().join(", ")
                )
            })?;
        let params = yeban_theory::DrumGenParams {
            complexity: request.complexity,
            intensity: request.intensity,
            swing_permille: request.swing_permille.or(Some(preset.swing_permille)),
            seed: request.seed,
            bars: request.bars,
            fill: request.fill,
        };
        let display = self
            .selected_track_id()
            .and_then(|id| self.project.tracks.get(&id));
        let target = bridge::drum_gen_target(&self.project, display).cloned();
        let ops = bridge::drum_gen_ops(&self.project, &preset, &params, target.as_ref())
            .map_err(|reason| format!("鼓机生成被拒绝：{reason}"))?;
        let note_count = ops
            .iter()
            .find_map(|op| match op {
                Op::AddClip { clip } => match &clip.content {
                    ClipContent::Midi { notes } => Some(notes.len()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or(0);
        let new_track = ops.iter().find_map(|op| match op {
            Op::AddTrack { track } if track.kind == TrackKind::Midi => Some(track.id),
            _ => None,
        });
        let destination = target.as_ref().map_or_else(
            || "新建鼓轨".to_owned(),
            |track| format!("音轨「{}」", track.name),
        );
        let clips_before = self.project.clip_pool.len();
        self.commit_edit(
            "tui: generate drum pattern",
            ops,
            &format!(
                "鼓机生成：{} · {} 小节 · {note_count} 个音符 → {destination}",
                preset.name, request.bars
            ),
        );
        if self.project.clip_pool.len() == clips_before {
            return Err("鼓机生成被拒绝：提交未落库（原因见底栏日志）".to_owned());
        }
        // 新建鼓轨 ⇒ 选中它（与 :track 同一条纪律：新对象立刻可编辑）。
        if let Some(id) = new_track
            && let Some(index) = self.track_ids.iter().position(|candidate| *candidate == id)
        {
            self.selected = index;
        }
        Ok(format!(
            "鼓机生成：{} · {} 小节 · {note_count} 个音符 → {destination}",
            preset.name, request.bars
        ))
    }

    /// 分轨导出（GUI「导出分轨 (Stems)...」与 `yeban_export_stems` 的 TUI 等价面）。
    ///
    /// 后端是 `yeban_app::export_stems::export_stems`（与 GUI 菜单臂、CLI
    /// `--export-stems` **同一份**）：`N + 1` 次渲染、逐轨 32-bit Float、文件名
    /// `{序号}.{清洗后的轨名}.wav`、跳过的轨逐条点名（⛔ 不静默丢轨）、
    /// `stems-manifest.json` 落同一目录。目录不存在时在写盘口创建（B6 口径）。
    ///
    /// # Errors
    ///
    /// 目录被一个非目录占着 / 目标文件已存在 / 渲染或写盘失败时返回
    /// `[稳定错误码] 原因`（原文照传，不吞）。
    pub fn export_stems(
        &mut self,
        dir: &Path,
    ) -> Result<yeban_app::export_stems::ExportStemsReport, String> {
        let project_path = self
            .project_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "分轨导出失败：既没有工程路径，也读不到当前目录".to_owned())?;
        let assets: Vec<(AssetHash, Vec<u8>)> = self
            .archive
            .as_ref()
            .map(|archive| archive.assets.clone())
            .unwrap_or_default();
        let report =
            yeban_app::export_stems::export_stems(&self.project, &project_path, dir, None, &assets)
                .map_err(|error| format!("分轨导出失败［{}］：{}", error.code(), error.reason()))?;
        // 回执行与 GUI / CLI 同一份（`ExportStemsReport::lines`），逐条进日志。
        for line in report.lines() {
            self.log_line(line);
        }
        Ok(report)
    }

    /// 诊断包导出（GUI 诊断导出与 `yeban_export_diagnostics` 的 TUI 等价面）。
    ///
    /// **与 GUI 逐项同**：同一个采集器 `yeban_diagnostics::export_diagnostics`、同一组
    /// 输入（`config.json` 的「不可用」在场记录；日志 / 崩溃 / 工程默认不带 —— 隐私默认）、
    /// 缺省目录同为 `std::env::temp_dir()`。包内条目因此与 GUI 一致：
    /// `env.txt` / `git.txt` / `config.json` / `MANIFEST.txt`。
    ///
    /// # Errors
    ///
    /// 目录不存在 / 不是目录 / 打包或写盘失败时返回原因文本。
    pub fn export_diagnostics(
        &mut self,
        out_dir: Option<&Path>,
    ) -> Result<DiagnosticsReceipt, String> {
        let dir = out_dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
        if !dir.is_dir() {
            return Err(format!(
                "诊断包导出失败：目录不存在 `{}`（先创建目录，或换一个已存在的路径）",
                dir.display()
            ));
        }
        let config = yeban_diagnostics::unavailable_config_json();
        let inputs = yeban_diagnostics::BundleInputs {
            config_json: Some(&config),
            ..Default::default()
        };
        let report = yeban_diagnostics::export_diagnostics(&dir, inputs)
            .map_err(|error| format!("诊断包导出失败：{error}"))?;
        let receipt = DiagnosticsReceipt {
            path: report.path.clone(),
            bytes: report.bytes,
            sha256: report.sha256.clone(),
            entries: report
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
        };
        self.log_line(format!(
            "诊断包已写出 {}（{} 字节 · sha256 {} · {} 个条目）",
            receipt.path.display(),
            receipt.bytes,
            receipt.sha256,
            receipt.entries.len()
        ));
        Ok(receipt)
    }

    /// 打开**设备腿**（GUI 启动时 `EngineHost::open_device` 的 TUI 等价面）：
    /// 音频线程改由真实设备时钟驱动，设备行随之报出设备名与两侧采样率。
    ///
    /// # Errors
    ///
    /// 设备腿已开 / 没有引擎 / 设备不可用（无声卡、采样率不支持、格式不是 `f32`）
    /// 时返回原文原因（⛔ 不静默降级成"看起来开了"）。
    pub fn open_device(&mut self) -> Result<String, String> {
        if self.host.device_active() {
            return Err("设备腿已经开着：先 :device off 再重开".to_owned());
        }
        let config = yeban_engine::device::EngineConfig::from_project(&self.project);
        match self.host.open_device(config) {
            Ok((opening, collector)) => {
                self.collector = Some(collector);
                let line = opening.status_line();
                self.log_line(line.clone());
                self.log_line(self.device_status_line());
                Ok(line)
            }
            Err(error) => Err(format!("打开音频设备失败：{error}")),
        }
    }

    /// 关掉设备腿并回到墙钟泵送（重建一代控制面引擎，与 `:reload` 同一落点）。
    ///
    /// # Errors
    ///
    /// 设备腿本来就没开时返回原因（⛔ 不假装关过一次）。
    pub fn close_device(&mut self) -> Result<String, String> {
        if !self.host.close_device() {
            return Err("设备腿本来就没开（TUI 走墙钟泵送）".to_owned());
        }
        self.reload_engine();
        Ok("已关设备腿：回到墙钟泵送".to_owned())
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

/// 文本缓冲里**不接受**的修饰键组合（Ctrl/Alt/Cmd/Win 都不是"输入一个字符"）。
///
/// T-FIX-86④：终端会把 `Cmd+V` / `Win+…` 也作为 `Char` 送进来，若当裸字符处理，
/// 文本缓冲会混进 `v`/`z` 之类的伪输入。⛔ 一个也不猜，全部拒绝。
fn is_modifier_leak(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(
        KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SUPER
            | KeyModifiers::META
            | KeyModifiers::HYPER,
    )
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
        // T-FIX-86④ 合成态防护：合成中除 F5/F6（全局视图键）外一律吞掉。
        // 与 `InputContext` 的规则同一句（解析器第 1/2 步），这里在**进解析器之前**
        // 再拦一次：文本缓冲/插件宿主都碰不到半成品按键。
        if self.input_ctx.is_composing() && !matches!(event.code, KeyCode::F(5) | KeyCode::F(6)) {
            return;
        }
        // T-FIX-86④ 修饰键泄漏防护：终端把 Cmd/Win（SUPER/META/HYPER）组合也送进来，
        // 若把它当"裸键"处理，`Cmd+V` 会被读成 `v`、`Cmd+Z` 被读成撤销（macOS 上尤其
        // 明显）。这些组合 TUI 一概不认，⛔ 不猜用户想按什么。
        if event
            .modifiers
            .intersects(KeyModifiers::SUPER | KeyModifiers::META | KeyModifiers::HYPER)
        {
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
        // 选中轨基准行高 ±16 px（= `MIN_TRACK_HEIGHT_PX`，`Alt+Shift+↑/↓`）。
        // Alt+Shift 在 `to_logical` 里没有对应构造子（GUI 表也没有这一格），
        // 因此它一定走到这里；生效值仍由 `effective_track_height_px` 统一夹紧。
        if event.modifiers.contains(KeyModifiers::ALT)
            && event.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(event.code, KeyCode::Up | KeyCode::Down)
        {
            let delta = if event.code == KeyCode::Down { -16 } else { 16 };
            self.nudge_track_height_px(delta);
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
            // 自动化视图：`a` 在光标处按当前生效值设点（T-FIX-76 第 4 项）。
            if matches!(self.list_view, Some(ListView::Automation { .. })) && ch == 'a' {
                self.vi.reset();
                self.add_automation_point_at_cursor();
                return;
            }
            // 打开自动化泳道：`P` = 声相（TUI 自加键；实测共享表里裸 `P`
            // 是 PassThrough —— GUI 的 `e`/`p` 只认小写；`Ctrl+Alt+A`
            // 走 GUI 的 `ToggleAutomation`，打开/关闭音量泳道）。
            if self.list_view.is_none() && ch == 'P' {
                self.vi.reset();
                self.open_automation_view(AutomationKind::Pan);
                return;
            }
            // 移轨（显示顺序）：`J`/`K`，计数前缀 = 格数。音符视图里 `J`/`K`
            // 是移音高（已在上面分流）；这里只接视图未开的情形。
            if self.list_view.is_none() && matches!(ch, 'J' | 'K') {
                let count = i32::try_from(self.vi.count().unwrap_or(1)).unwrap_or(1);
                self.vi.reset();
                self.move_selected_track(if ch == 'J' { count } else { -count });
                return;
            }
            // 全局高度乘子：`{`/`}` 每档 10%，计数前缀 = 档数（TUI 自加键；
            // 裸 `[`/`]` 在共享表里是试听，⛔ 不抢）。
            if self.list_view.is_none() && matches!(ch, '{' | '}') {
                let count = i32::try_from(self.vi.count().unwrap_or(1)).unwrap_or(1);
                self.vi.reset();
                let step = 10_i32.saturating_mul(count);
                self.nudge_height_percent(if ch == '}' { step } else { -step });
                return;
            }
            // 片段网格焦点下开音符视图（TUI 自加键）：裸 `n`（共享表
            // PassThrough、Vi 层未占）。视图已开时不重复开。
            if ch == 'n' && self.focus == Pane::Grid && self.list_view.is_none() {
                self.vi.reset();
                self.open_notes_view();
                return;
            }
            if ch == 'n' && self.focus != Pane::Grid && self.list_view.is_none() {
                self.vi.reset();
                self.log_line("先 Tab 到片段网格，再按 n 开音符视图".to_owned());
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
            // GUI `F5`/`F6`（与 `ui/switch_main_view` 同一对白名单值）：切主视图。
            Action::ShowView(view) => self.set_view(view),
            // GUI `Ctrl+Shift+A`（`ToggleAutomation`：泳道可见性开关）→ TUI 的
            // 等价形态是打开/关闭音量泳道的列表视图（同一"显示自动化"语义）。
            // ⚠ 实测 `ctrl_alt+a` 在共享表里是 `ToggleAiDrawer`，`ctrl_shift+a`
            // 才是 `ToggleAutomation` —— 键位以解析器为准，不靠猜。
            Action::ToggleAutomation => self.toggle_automation_view(),
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
                } else if matches!(self.list_view, Some(ListView::Automation { .. })) {
                    // 自动化视图：`+` 是值 +1 档（音量 0.5 dB / 声相 0.05）。
                    self.nudge_automation_value(1);
                } else {
                    self.bar_cols = (self.bar_cols + 1).min(8);
                }
            }
            Action::ZoomOut => {
                if matches!(self.list_view, Some(ListView::Notes { .. })) {
                    self.nudge_selected_note_velocity(-10);
                } else if matches!(self.list_view, Some(ListView::Automation { .. })) {
                    self.nudge_automation_value(-1);
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
            Action::ResizeSelectionDuration { direction } => {
                // Shift+←/→（UI-NOTE-005 项 3）：音符视图内改选中音符时值；
                // 换算与 op 组装都走 GUI 的同一对纯函数（同一吸附网格）。
                self.resize_selected_note_by_grid(direction);
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
                    Some(ListView::Automation { .. }) => self.remove_automation_point_at_cursor(),
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
            self.log_line("改名中：Ctrl+U 清空 · Enter 提交 · Esc 取消".to_owned());
            return;
        }
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get(&id) {
            self.rename = Some(track.name.clone());
            self.rename_target = Some(RenameTarget::Track(id));
            self.mode = Mode::Insert;
            self.log_line("改名中：Ctrl+U 清空 · Enter 提交 · Esc 取消".to_owned());
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
            KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(buffer) = &mut self.rename {
                    buffer.clear();
                }
            }
            KeyCode::Char(ch) => {
                if is_modifier_leak(event.modifiers) {
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

    /// 按名字过滤「最近打开工程」（saved sessions 在磁盘上的落点是本机配置的
    /// `recent_projects`，显示名从路径派生，⛔ 不在这里另存一份会漂移的名字）。
    /// 空 query 列出全部；大小写不敏感匹配文件名（不含扩展名）。
    fn filter_recent_sessions(
        entries: &[yeban_app::settings::RecentProject],
        query: &str,
    ) -> Vec<PathBuf> {
        let q = query.trim().to_lowercase();
        entries
            .iter()
            .filter(|entry| {
                if q.is_empty() {
                    return true;
                }
                entry
                    .path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_lowercase().contains(&q))
            })
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// 把候选工程列成编号行；空列表 ⇒ 一条响亮报错，绝不静默。
    fn session_list_lines(candidates: &[PathBuf], query: &str) -> Vec<String> {
        if candidates.is_empty() {
            return vec![format!(
                "没有匹配的已存工程：{:?}（用 :sessions 不带参数列出全部）",
                query
            )];
        }
        let mut lines = Vec::with_capacity(candidates.len() + 1);
        lines.push(format!(
            "已存工程（{} 条，:sessions open[!] <序号> 打开）：",
            candidates.len()
        ));
        for (idx, path) in candidates.iter().enumerate() {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("<无效名字>");
            lines.push(format!("  {}）{} — {}", idx + 1, name, path.display()));
        }
        lines
    }

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
                if is_modifier_leak(event.modifiers) {
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
            "w!" => {
                // 强制保存（GUI `ui/force_save` 的等价物）：与 `:w` 同一个落盘路径，
                // 语义差别只在"强制"这件事被显式说出来（无路径时照样响亮报错）。
                match self.force_save() {
                    Ok(done) => self.log_line(done),
                    Err(error) => self.log_line(error),
                }
            }
            "reload" | "reload-engine" => self.reload_engine(),
            "play" => {
                self.host.play();
                self.log_line("播放".to_owned());
            }
            "seek" => {
                // :seek <bar> —— 播放头定位到第 bar 小节（1 起，与标尺及顶栏读数一致）。
                match rest.parse::<usize>() {
                    Ok(0) => {
                        self.log_line("小节号从 1 起：用法 :seek <小节号（1 起）>".to_owned());
                    }
                    Ok(bar) => self.seek_to_bar(bar - 1),
                    Err(_) => self.log_line("用法：:seek <小节号（1 起）>".to_owned()),
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
            "view" => {
                // :view session|arrangement —— 与 F5/F6 同一落点（View 白名单同一个）。
                match rest {
                    "session" => self.set_view(View::Session),
                    "arrangement" => self.set_view(View::Arrangement),
                    "" => self.log_line("用法：:view session|arrangement".to_owned()),
                    other => {
                        self.log_line(format!("用法：:view session|arrangement（收到 `{other}`）"));
                    }
                }
            }
            "move" => {
                // :move <位置> —— 显示顺序**绝对下标**（0 起，与 `newIndex` 同口径）。
                match rest.parse::<usize>() {
                    Ok(index) => self.move_selected_track_to(index),
                    Err(_) => self.log_line(
                        "用法：:move <位置>（0 起，与 yeban_move_track 的 newIndex 同口径）"
                            .to_owned(),
                    ),
                }
            }
            "height" => {
                // :height <px> | auto —— 选中轨**基准**高（auto/0 = 取消覆盖，
                // 与 `ui/set_track_height` 的 heightPx = 0 同一语义）。
                if rest.is_empty() {
                    self.log_line("用法：:height <px>（0 或 auto = 取消覆盖）".to_owned());
                } else if matches!(rest, "auto" | "0") {
                    self.set_track_height_px(0);
                } else {
                    match rest.parse::<u32>() {
                        Ok(px) => self.set_track_height_px(px),
                        Err(_) => {
                            self.log_line("用法：:height <px>（0 或 auto = 取消覆盖）".to_owned());
                        }
                    }
                }
            }
            "height%" => {
                // :height% <n> —— 全局高度乘子（与 GUI 的 `percent` 同一个旋钮）。
                match rest.parse::<u32>() {
                    Ok(percent) => self.set_height_percent(percent),
                    Err(_) => self.log_line("用法：:height% <百分比，100 = 不缩放>".to_owned()),
                }
            }
            "auto" => {
                // :auto —— 六类目标的泳道清单 / 打开 / 新建（见 `command_automation`）。
                self.command_automation(rest);
            }
            "export" => {
                // 形态：export wav <path> 或 export <path>（默认 wav）。
                // ⚠ `:export wav`（不带路径）必须报用法，⛔ 不能把 "wav" 当文件名写盘。
                let words: Vec<&str> = rest.split_whitespace().collect();
                let target = match words.as_slice() {
                    ["wav"] | [] => None,
                    ["wav", path] | [path] => Some(*path),
                    _ => None,
                };
                match target {
                    None => self.log_line("用法：:export wav <path>".to_owned()),
                    Some(target) => {
                        if let Err(error) = self.export_wav(Path::new(target)) {
                            self.log_line(error);
                        }
                    }
                }
            }
            "stems" => {
                // :stems <dir> —— 一次导出 N 条轨（GUI 菜单 / CLI / MCP 同一份后端）。
                if rest.is_empty() {
                    self.log_line("用法：:stems <输出目录>".to_owned());
                } else if let Err(error) = self.export_stems(Path::new(rest)) {
                    self.log_line(error);
                }
            }
            "diag" | "diagnostics" => {
                // :diag [dir] —— 诊断包（缺省系统临时目录，与 GUI 同一落点）。
                let dir = if rest.is_empty() {
                    None
                } else {
                    Some(Path::new(rest))
                };
                if let Err(error) = self.export_diagnostics(dir) {
                    self.log_line(error);
                }
            }
            "drum" => {
                // :drum <风格> [小节] [复杂度] [强度] [摇摆] [种子] [fill|nofill]
                // 缺省与 GUI 同一口径：4 小节 / 50 / 80 / 风格定值 / 0 / fill 开。
                let words: Vec<&str> = rest.split_whitespace().collect();
                let usage = || {
                    format!(
                        "用法：:drum <风格> [小节 1..=16] [复杂度 0..=100] [强度 0..=100] \
                         [摇摆 500..=1000] [种子] [fill|nofill]（出厂风格：{}）",
                        yeban_theory::factory_drum_style_ids().join(", ")
                    )
                };
                if words.is_empty() {
                    self.log_line(usage());
                } else {
                    let style = words[0];
                    let mut numbers: Vec<u64> = Vec::new();
                    let mut fill = true;
                    let mut parse_failed = None;
                    for word in &words[1..] {
                        match *word {
                            "fill" => fill = true,
                            "nofill" => fill = false,
                            other => match other.parse::<u64>() {
                                Ok(value) => numbers.push(value),
                                Err(_) => {
                                    parse_failed = Some(other.to_owned());
                                    break;
                                }
                            },
                        }
                    }
                    let bars = numbers.first().copied().unwrap_or(4);
                    let complexity = numbers.get(1).copied().unwrap_or(50);
                    let intensity = numbers.get(2).copied().unwrap_or(80);
                    let swing = numbers.get(3).copied();
                    let seed = numbers.get(4).copied().unwrap_or(0);
                    if let Some(word) = parse_failed {
                        self.log_line(format!("`{word}` 不是数字：{}", usage()));
                    } else if numbers.len() > 5 {
                        self.log_line(usage());
                    } else if !(1..=16).contains(&bars) {
                        self.log_line(format!("`bars` 必须是 1..=16 的整数, 实际收到 {bars}"));
                    } else if complexity > 100 {
                        self.log_line(format!(
                            "`complexity` 必须是 0..=100 的整数, 实际收到 {complexity}"
                        ));
                    } else if intensity > 100 {
                        self.log_line(format!(
                            "`intensity` 必须是 0..=100 的整数, 实际收到 {intensity}"
                        ));
                    } else if let Some(swing) = swing
                        && !(500..=1000).contains(&swing)
                    {
                        self.log_line(format!(
                            "`swingPermille` 必须是 500..=1000 的整数, 实际收到 {swing}"
                        ));
                    } else {
                        let request = DrumRequest {
                            style: style.to_owned(),
                            bars: u32::try_from(bars).unwrap_or(u32::MAX),
                            complexity: u8::try_from(complexity).unwrap_or(u8::MAX),
                            intensity: u8::try_from(intensity).unwrap_or(u8::MAX),
                            swing_permille: swing
                                .map(|value| u16::try_from(value).unwrap_or(u16::MAX)),
                            seed,
                            fill,
                        };
                        if let Err(error) = self.generate_drum_pattern(&request) {
                            self.log_line(error);
                        }
                    }
                }
            }
            "a11y" | "semantics" => {
                // :a11y —— 语义清单（角色 / ID / 序 / 标签；与 GUI 同一套角色词）。
                for line in self.semantic_lines() {
                    self.log_line(line);
                }
            }
            "info" | "clock" => {
                // :info —— 工程信息面（契约 / 960 PPQ 时钟 / 身份 / 规模）。
                for line in self.project_info_lines() {
                    self.log_line(line);
                }
            }
            "device" => match rest {
                // :device —— 设备状态行（名 / 设备率 / 工程率 / 转换状态）。
                "" | "status" => self.log_line(self.device_status_line()),
                "on" => {
                    if let Err(error) = self.open_device() {
                        self.log_line(error);
                    }
                }
                "off" => match self.close_device() {
                    Ok(line) => self.log_line(line),
                    Err(error) => self.log_line(error),
                },
                other => self.log_line(format!("用法：:device [on|off|status]（收到 `{other}`）")),
            },
            "track" => {
                // :track [midi|audio|aux] —— 新建一条轨并选中它。
                // 配方与 GUI 新建轨同源（`bridge::track_create_ops`），经撤销端口提交。
                let parsed = match rest {
                    "midi" => Some((NewTrackKind::Midi, "MIDI")),
                    "audio" => Some((NewTrackKind::Audio, "音频")),
                    "aux" => Some((NewTrackKind::AuxReturn, "辅助")),
                    _ => None,
                };
                match parsed {
                    None => {
                        let usage = if rest.is_empty() {
                            "用法：:track [midi|audio|aux]".to_owned()
                        } else {
                            format!("用法：:track [midi|audio|aux]（收到 `{rest}`）")
                        };
                        self.log_line(usage);
                    }
                    Some((kind, label)) => match bridge::track_create_ops(&self.project, kind) {
                        Ok(ops) => {
                            let new_id = ops.iter().find_map(|op| match op {
                                Op::AddTrack { track } => Some(track.id),
                                _ => None,
                            });
                            self.commit_edit("tui: create track", ops, &format!("新建{label}轨"));
                            if let Some(id) = new_id
                                && let Some(index) =
                                    self.track_ids.iter().position(|candidate| *candidate == id)
                            {
                                self.selected = index;
                                let name = self
                                    .project
                                    .tracks
                                    .get(&id)
                                    .map(|track| track.name.clone())
                                    .unwrap_or_default();
                                self.log_line(format!("已新建{label}轨（`{name}`）"));
                            }
                        }
                        Err(reason) => self.log_line(format!("新建{label}轨失败：{reason}")),
                    },
                }
            }
            "clip" => {
                // :clip —— 在选中轨的**光标小节**新建 4 小节 MIDI 片段。
                // ⚠ 配方 `clip_create_ops_on` 固定 `start_tick: 0`；落点在这里改写到
                // `cursor_bar * bar_ticks()`，否则每次都在第 1 小节叠一条。
                if let Some(track_id) = self.selected_track_id() {
                    if let Some(track) = self.project.tracks.get(&track_id) {
                        match bridge::clip_create_ops_on(&self.project, Some(track)) {
                            Ok(mut ops) => {
                                let start_tick = u64::try_from(self.cursor_bar)
                                    .unwrap_or(u64::MAX)
                                    .saturating_mul(self.bar_ticks());
                                for op in &mut ops {
                                    if let Op::AddClipPlacement { placement, .. } = op {
                                        placement.start_tick = start_tick;
                                    }
                                }
                                self.commit_edit(
                                    "tui: create clip",
                                    ops,
                                    &format!("在第 {} 小节新建片段", self.cursor_bar + 1),
                                );
                            }
                            Err(reason) => self.log_line(format!("新建片段被拒绝：{reason}")),
                        }
                    } else {
                        self.log_line("选中的轨已不在工程里：先重新选一条轨".to_owned());
                    }
                } else {
                    self.log_line("先选中一条轨道，再 :clip".to_owned());
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
            "sessions" | "sessions!" => {
                // `:sessions open[!] <n>` 从上次列出的结果里打开；`open! ` 与
                // `:sessions!` 都表示强开（有未保存修改时也打开）。`open ` /
                // `open! ` 后不是合法序号时，整个 rest 当作 query 回落查询
                // （工程名可能以 open 开头）；只有数字但越界时才报越界。
                let open = rest
                    .strip_prefix("open! ")
                    .map(|arg| (arg, true))
                    .or_else(|| rest.strip_prefix("open ").map(|arg| (arg, false)));
                let open_num = open.and_then(|(arg, open_bang)| {
                    arg.trim().parse::<usize>().ok().map(|n| (n, open_bang))
                });
                match open_num {
                    Some((n, open_bang)) => {
                        if n == 0 || n > self.session_search.len() {
                            self.log_line(format!(
                                "无效序号：{n}（先 :sessions 列出，再 :sessions open[!] <序号>）"
                            ));
                        } else {
                            let force = verb == "sessions!" || open_bang;
                            let path = self.session_search[n - 1].clone();
                            if self.dirty && !force {
                                self.log_line(format!(
                                    "有未保存修改：先 :w 保存，或 :sessions open! {n} 强开"
                                ));
                            } else {
                                match Self::open_file(&path) {
                                    Ok(next) => *self = next,
                                    Err(error) => self.log_line(error),
                                }
                            }
                        }
                    }
                    None => {
                        let settings = yeban_app::settings::load();
                        let candidates =
                            Self::filter_recent_sessions(&settings.recent_projects, rest);
                        self.session_search = candidates.clone();
                        for line in Self::session_list_lines(&candidates, rest) {
                            self.log_line(line);
                        }
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
    use std::sync::Mutex;

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
    fn tick_publishes_automation_to_the_engine() {
        let mut app = App::with_demo().expect("demo app");
        let before = app.host.automation_counts();
        app.tick();
        let after = app.host.automation_counts();
        assert!(
            after.0 > before.0,
            "TUI 的 tick 必须每跳调用 publish_automation（before={before:?}, after={after:?}）"
        );
        assert_eq!(
            app.host
                .automation_project()
                .map(|project| project.title.as_str()),
            Some(app.project.title.as_str()),
            "首跳必须把当前工程交给自动化缓存"
        );
    }

    #[test]
    fn automation_selection_edits_and_deletes_the_exact_tick() {
        let mut app = App::with_demo().expect("demo app");
        app.selected = 0;
        app.cursor_bar = 0;
        app.open_automation_view(AutomationKind::Volume);
        let points = app.automation_points();
        assert_eq!(points.len(), 3);
        assert_eq!(points[1].tick, 1920, "演示轨 0 的中间点应在半小节");
        assert_eq!(app.cursor_bar, 0);
        let point_at_zero = points[0];
        let point_at_half_bar = points[1];

        // 选中半小节里的点：小节光标只能到小节边界（0），但读数与编辑必须用精确 tick。
        app.view_move_selection(1);
        assert_eq!(app.cursor_bar, 0, "小节光标仍是 0 —— 这正是截断现场");
        let readout = app.automation_readout().expect("读数");
        assert_eq!(readout.tick, 1920);
        assert_eq!(readout.automation_value, Some(point_at_half_bar.value));

        app.nudge_automation_value(1);
        let after = app.automation_points();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0].tick, point_at_zero.tick, "tick=0 的点必须原样保留");
        assert_eq!(after[0].value, point_at_zero.value);
        assert_eq!(after[1].tick, 1920);
        assert!(
            after[1].value > point_at_half_bar.value,
            "`+` 必须改 1920 的点，而不是 0 的点"
        );

        // Delete 也必须按选中点的精确 tick 删，不能回到小节起点删 tick=0。
        app.remove_automation_point_at_cursor();
        let after_delete = app.automation_points();
        assert_eq!(after_delete.len(), 2);
        assert!(
            after_delete.iter().all(|point| point.tick != 1920),
            "被删的必须是选中的 1920 点：{after_delete:?}"
        );
        assert_eq!(after_delete[0].tick, 0);
        assert_eq!(after_delete[0].value, point_at_zero.value);
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

    #[test]
    fn position_bar_beat_and_playhead_index_are_consistent() {
        let mut app = App::with_demo().expect("demo app");
        // 控制面墙钟泵送自宿主启动起推进播放头，初始位置不保证恰为 0；
        // 先 :seek 1 把位置钉到 0，再断言 1 起口径的一致性。
        app.execute_command("seek 1");
        assert_eq!(app.position_bar_beat(), (1, 1, 0));
        assert_eq!(app.playhead_bar_index(), 0);
        assert_eq!(app.cursor_bar, 0);

        app.execute_command("seek 1");
        assert_eq!(app.position_bar_beat(), (1, 1, 0));
        assert_eq!(app.playhead_bar_index(), 0);
        assert_eq!(app.cursor_bar, 0);
        assert_eq!(app.transport().position_ticks, 0);

        app.execute_command("seek 3");
        assert_eq!(app.position_bar_beat(), (3, 1, 0));
        assert_eq!(app.playhead_bar_index(), 2);
        assert_eq!(app.cursor_bar, 2);
        assert_eq!(app.transport().position_ticks, 2 * app.bar_ticks());
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("第 3 小节"))
        );

        let mid_ticks = app.bar_ticks() + app.ticks_per_beat() + 123;
        app.host
            .send_transport(&[TransportCommand::SeekTicks(mid_ticks)]);
        assert_eq!(app.position_bar_beat(), (2, 2, 123));
        assert_eq!(app.playhead_bar_index(), 1);
    }

    #[test]
    fn seek_zero_and_invalid_args_are_rejected_without_moving_state() {
        let mut app = App::with_demo().expect("demo app");
        app.execute_command("seek 2");
        assert_eq!(app.cursor_bar, 1);
        assert_eq!(app.transport().position_ticks, app.bar_ticks());

        app.execute_command("seek 0");
        assert_eq!(app.cursor_bar, 1);
        assert_eq!(app.transport().position_ticks, app.bar_ticks());
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("小节号从 1 起"))
        );

        for cmd in ["seek -1", "seek abc", "seek"] {
            app.execute_command(cmd);
            assert_eq!(app.cursor_bar, 1, "{cmd} should not move cursor_bar");
            assert_eq!(
                app.transport().position_ticks,
                app.bar_ticks(),
                "{cmd} should not move playhead"
            );
            assert!(
                app.log
                    .last()
                    .is_some_and(|line| line.contains("用法：:seek")),
                "{cmd} should log usage, got {:?}",
                app.log.last()
            );
        }
    }

    #[test]
    fn seek_large_bar_does_not_overflow_or_panic() {
        let mut app = App::with_demo().expect("demo app");
        app.execute_command(&format!("seek {}", usize::MAX));
        // 合入口径：:seek 沿用 TUI 修复包的钳位语义——超界定位到最后一小节
        // 并响亮告知工程总小节数；超大入参不得溢出或 panic。
        let total = app.total_bars();
        assert_eq!(app.cursor_bar, total - 1);
        assert_eq!(
            app.transport().position_ticks,
            (total as u64 - 1) * app.bar_ticks()
        );
        assert!(
            app.log.last().is_some_and(|line| line.contains("工程只有")),
            "clamp should be reported loudly, got {:?}",
            app.log.last()
        );
    }

    fn recent_fixture() -> Vec<yeban_app::settings::RecentProject> {
        use yeban_app::settings::RecentProject;
        vec![
            RecentProject {
                path: PathBuf::from("/projects/Alpha Session.yeban"),
                opened_at_unix_ms: 3,
            },
            RecentProject {
                path: PathBuf::from("/projects/beta-demo.yeban"),
                opened_at_unix_ms: 2,
            },
            RecentProject {
                path: PathBuf::from("/other/Alpha Copy.yeban"),
                opened_at_unix_ms: 1,
            },
        ]
    }

    #[test]
    fn session_query_matches_names_case_insensitively() {
        let found = App::filter_recent_sessions(&recent_fixture(), "alpha");
        assert_eq!(found.len(), 2, "Alpha Session + Alpha Copy");
        assert!(found.iter().any(|p| p.ends_with("Alpha Session.yeban")));
        assert!(found.iter().any(|p| p.ends_with("Alpha Copy.yeban")));
        assert!(!found.iter().any(|p| p.ends_with("beta-demo.yeban")));

        // 文件名匹配、路径目录名不算（/other/ 不该命中 beta 的查询）。
        let found = App::filter_recent_sessions(&recent_fixture(), "BETA");
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("beta-demo.yeban"));
    }

    #[test]
    fn session_empty_query_lists_all_in_recent_order() {
        let found = App::filter_recent_sessions(&recent_fixture(), "   ");
        assert_eq!(found.len(), 3, "empty query lists all");
        // 顺序即 recent 顺序（最近在前），不重排。
        assert!(found[0].ends_with("Alpha Session.yeban"));
        assert!(found[1].ends_with("beta-demo.yeban"));
        assert!(found[2].ends_with("Alpha Copy.yeban"));
        let lines = App::session_list_lines(&found, "");
        assert!(lines[0].contains("3 条"));
        assert!(lines.iter().any(|l| l.contains("1）Alpha Session")));
    }

    #[test]
    fn session_no_match_reports_loudly() {
        let found = App::filter_recent_sessions(&recent_fixture(), "nothing-matches-this");
        assert!(found.is_empty(), "no match must yield empty candidates");
        let lines = App::session_list_lines(&found, "nothing-matches-this");
        assert_eq!(lines.len(), 1, "no-match must report exactly one loud line");
        assert!(lines[0].contains("没有匹配的已存工程"), "got {:?}", lines);

        // 端到端：命令也走同一条响亮路径（settings 读空 ⇒ 无匹配时留痕）。
        let mut app = App::with_demo().expect("demo app");
        app.session_search = Vec::new();
        // 直接验证 no-match 的播报文本不为空、且不是静默。
        for line in lines {
            app.log_line(line);
        }
        assert!(
            app.log
                .last()
                .is_some_and(|l| l.contains("没有匹配的已存工程"))
        );
    }

    /// T-FIX-245：设置路径的环境变量守卫。`YEBAN_SETTINGS_PATH` 是进程级
    /// 变量，判据期间必须独占（`onboarding_help.rs` 同款 `ENV_LOCK`），
    /// 否则并行测试会互相串改最近清单。
    static SETTINGS_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 把 `YEBAN_SETTINGS_PATH` 指到临时文件，Drop 时恢复原值。
    /// 调用方必须先持有 [`SETTINGS_ENV_LOCK`]。
    struct TempSettingsPath {
        previous: Option<std::ffi::OsString>,
    }

    impl TempSettingsPath {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV);
            // SAFETY: 调用方持有 SETTINGS_ENV_LOCK；见 `onboarding_help.rs` 同款注释。
            unsafe {
                std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, path);
            }
            Self { previous }
        }
    }

    impl Drop for TempSettingsPath {
        fn drop(&mut self) {
            // SAFETY: 守卫在 SETTINGS_ENV_LOCK 释放前析构，恢复进入时读到的原值。
            unsafe {
                match &self.previous {
                    Some(value) => {
                        std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, value);
                    }
                    None => std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV),
                }
            }
        }
    }

    fn temp_settings_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yeban-tui-recent-{tag}-{}.json",
            std::process::id()
        ))
    }

    struct TempFileCleanup(PathBuf);
    impl Drop for TempFileCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn session_open_chosen_index_loads_that_project() {
        // T-FIX-245：`:sessions open` 走 `open_file`，会回写最近清单；
        // 指到临时设置文件，避免判据碰真实配置目录。
        let _lock = SETTINGS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let settings_path = temp_settings_path("sessions-open");
        let _settings_cleanup = TempFileCleanup(settings_path.clone());
        let _env = TempSettingsPath::set(&settings_path);
        struct TempCleanup(PathBuf);
        impl Drop for TempCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let saved =
            std::env::temp_dir().join(format!("yeban-tui-sessions-{}.yeban", std::process::id()));
        let _cleanup = TempCleanup(saved.clone());

        let mut writer = App::with_demo().expect("demo app");
        writer.save_to(&saved).expect("save demo as saved session");

        let mut app = App::with_demo().expect("demo app");
        // 模拟 :sessions 已列出两条，选中第 1 条打开（与既有 :open 同一条落盘路径）。
        app.session_search = vec![saved.clone(), PathBuf::from("/nonexistent/second.yeban")];
        app.execute_command("sessions open 1");
        assert_eq!(app.project_path.as_deref(), Some(saved.as_path()));
        assert!(!app.dirty, "opened session is clean");
    }

    /// T-FIX-246：把 `YEBAN_SETTINGS_PATH` 指到临时文件，串行化同进程测试。
    /// `std::env::set_var` 是进程级全局量，测试线程并行跑时必须持锁，否则
    /// 别的测试可能读到这份临时设置。
    struct TempSettings {
        _guard: std::sync::MutexGuard<'static, ()>,
        path: PathBuf,
        old: Option<std::ffi::OsString>,
    }

    impl TempSettings {
        fn new() -> Self {
            let guard = SETTINGS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let path = std::env::temp_dir().join(format!(
                "yeban-tui-sessions-settings-{}.json",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            let old = std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV);
            // SAFETY: 全程持有 SETTINGS_ENV_LOCK，本进程内没有其它线程在
            // 这段时间读写该环境变量；测试结束在 Drop 里恢复原值。
            unsafe { std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, &path) };
            Self {
                _guard: guard,
                path,
                old,
            }
        }
    }

    impl Drop for TempSettings {
        fn drop(&mut self) {
            match &self.old {
                Some(old) => {
                    // SAFETY: 与 new() 同锁（guard 仍在手），恢复自己设置前的值。
                    unsafe { std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, old) }
                }
                None => {
                    // SAFETY: 与 new() 同锁（guard 仍在手），删除自己设置的值。
                    unsafe { std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV) }
                }
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    struct TempDirCleanup(PathBuf);

    impl Drop for TempDirCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// T-FIX-246：脏工程 `:sessions open! 1` 真强开；`:sessions! open 1`
    /// 同样强开；不带 `!` 时脏检查仍拦住。
    #[test]
    fn sessions_open_bang_force_opens_dirty_project() {
        // 打开会回写最近清单（T-FIX-245），持锁指到临时设置文件隔离。
        let _settings = TempSettings::new();
        struct TempCleanup(PathBuf);
        impl Drop for TempCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let saved = std::env::temp_dir().join(format!(
            "yeban-tui-sessions-bang-{}.yeban",
            std::process::id()
        ));
        let _cleanup = TempCleanup(saved.clone());
        let mut writer = App::with_demo().expect("demo app");
        writer.save_to(&saved).expect("save demo as saved session");

        // `:sessions open! 1` 强开脏工程。
        let mut app = App::with_demo().expect("demo app");
        app.session_search = vec![saved.clone()];
        app.dirty = true;
        app.execute_command("sessions open! 1");
        assert_eq!(app.project_path.as_deref(), Some(saved.as_path()));
        assert!(!app.dirty, "forced open loads a clean session");

        // `:sessions! open 1` 同样强开。
        let mut app = App::with_demo().expect("demo app");
        app.session_search = vec![saved.clone()];
        app.dirty = true;
        app.execute_command("sessions! open 1");
        assert_eq!(app.project_path.as_deref(), Some(saved.as_path()));
        assert!(!app.dirty, "sessions! verb also forces open");

        // 不带 `!` 时脏检查仍拦住，不打开。
        let mut app = App::with_demo().expect("demo app");
        app.session_search = vec![saved.clone()];
        app.dirty = true;
        app.execute_command("sessions open 1");
        assert_ne!(app.project_path.as_deref(), Some(saved.as_path()));
        assert!(app.dirty, "plain open must not discard dirty edits");
        assert!(
            app.log.iter().any(|line| line.contains(":sessions open!")),
            "dirty block must point at the force form, got {:?}",
            app.log
        );
    }

    /// T-FIX-245：`App::open_file` 成功打开后回写最近工程清单，
    /// `:sessions` 重新 `settings::load()` 必须列出它。
    #[test]
    fn open_file_records_recent_project_and_sessions_lists_it() {
        let _lock = SETTINGS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let settings_path = temp_settings_path("open");
        let _settings_cleanup = TempFileCleanup(settings_path.clone());
        let _env = TempSettingsPath::set(&settings_path);

        let saved = std::env::temp_dir().join(format!(
            "yeban-tui-recent-open-{}.yeban",
            std::process::id()
        ));
        let _cleanup = TempFileCleanup(saved.clone());

        let mut writer = App::with_demo().expect("demo app");
        writer.save_to(&saved).expect("save demo as temp project");

        let mut app = App::open_file(&saved).expect("open temp project");
        assert_eq!(app.project_path.as_deref(), Some(saved.as_path()));

        let settings = yeban_app::settings::load();
        assert!(
            settings
                .recent_projects
                .first()
                .is_some_and(|entry| entry.path == saved),
            "recent_projects first must be the opened project, got {:?}",
            settings.recent_projects
        );

        app.execute_command("sessions");
        let stem = saved
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("temp project stem");
        assert!(
            app.log.iter().any(|line| line.contains(stem)),
            ":sessions must list the opened project, log: {:?}",
            app.log
        );
    }

    /// T-FIX-246：`open ` 后是非数字参数时回落查询、不报序号错；
    /// 以 `open ` 开头的工程名能被 `:sessions open <词>` 搜到。
    #[test]
    fn sessions_open_non_numeric_falls_back_to_query() {
        let _settings = TempSettings::new();
        let dir =
            std::env::temp_dir().join(format!("yeban-tui-sessions-query-{}", std::process::id()));
        let _dir_cleanup = TempDirCleanup(dir.clone());
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let open_alpha = dir.join("open alpha.yeban");
        let open_session = dir.join("open session.yeban");
        for path in [&open_alpha, &open_session] {
            let mut writer = App::with_demo().expect("demo app");
            writer.save_to(path).expect("save demo as saved session");
            yeban_app::settings::record_recent_project(path, 1);
        }

        let mut app = App::with_demo().expect("demo app");
        app.execute_command("sessions open alpha");
        assert!(
            !app.log.iter().any(|line| line.contains("无效序号")),
            "non-numeric open arg must fall back to query, got {:?}",
            app.log
        );
        assert_eq!(app.session_search, vec![open_alpha.clone()]);

        let mut app = App::with_demo().expect("demo app");
        app.execute_command("sessions open session");
        assert!(
            !app.log.iter().any(|line| line.contains("无效序号")),
            "open-prefixed name search must not report a serial error, got {:?}",
            app.log
        );
        assert_eq!(app.session_search, vec![open_session.clone()]);

        // `open! ` 后是非数字同样回落查询，不报序号错。
        let mut app = App::with_demo().expect("demo app");
        app.execute_command("sessions open! alpha");
        assert!(
            !app.log.iter().any(|line| line.contains("无效序号")),
            "non-numeric open! arg must also fall back to query, got {:?}",
            app.log
        );
    }

    /// T-FIX-246：数字越界仍报越界，不回落成查询。
    #[test]
    fn sessions_open_out_of_range_still_reports_range_error() {
        let _settings = TempSettings::new();
        struct TempCleanup(PathBuf);
        impl Drop for TempCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let saved = std::env::temp_dir().join(format!(
            "yeban-tui-sessions-range-{}.yeban",
            std::process::id()
        ));
        let _cleanup = TempCleanup(saved.clone());
        let mut writer = App::with_demo().expect("demo app");
        writer.save_to(&saved).expect("save demo as saved session");

        let mut app = App::with_demo().expect("demo app");
        app.session_search = vec![saved.clone()];
        app.execute_command("sessions open 5");
        assert!(
            app.log.iter().any(|line| line.contains("无效序号")),
            "out-of-range number must still report a range error, got {:?}",
            app.log
        );
        assert_ne!(app.project_path.as_deref(), Some(saved.as_path()));
        assert_eq!(
            app.session_search,
            vec![saved.clone()],
            "range error must not relist as a query"
        );

        let mut app = App::with_demo().expect("demo app");
        app.session_search = vec![saved.clone()];
        app.execute_command("sessions open 0");
        assert!(
            app.log.iter().any(|line| line.contains("无效序号")),
            "zero must still report a range error, got {:?}",
            app.log
        );
    }

    /// T-FIX-245：演示工程不走 `open_file`，不得进入最近清单。
    #[test]
    fn with_demo_does_not_record_recent_project() {
        let _lock = SETTINGS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let settings_path = temp_settings_path("demo");
        let _settings_cleanup = TempFileCleanup(settings_path.clone());
        let _env = TempSettingsPath::set(&settings_path);

        let _app = App::with_demo().expect("demo app");
        let settings = yeban_app::settings::load();
        assert!(
            settings.recent_projects.is_empty(),
            "demo must not be recorded, got {:?}",
            settings.recent_projects
        );
    }

    /// T-FIX-160：保存前必须把当前音符实际触发的样本收进归档；旧实现只搬
    /// `archive.assets`，新增触发样本永远不进容器。
    #[test]
    fn save_collects_new_triggered_samples_into_the_archive() {
        use std::fs;

        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        let root = std::env::temp_dir().join(format!(
            "yeban-tui-embed-save-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _cleanup = TempDir(root.clone());
        let kit = root.join("proj/assets/samples/kit");
        fs::create_dir_all(&kit).expect("建采样目录");
        fs::write(kit.join("001.sfz"), "<region> sample=tone60.wav key=60\n").expect("写 SFZ");
        let frames = [0_i16; 4800];
        let mut wav = Vec::with_capacity(44 + frames.len() * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + (frames.len() * 2) as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&48_000_u32.to_le_bytes());
        wav.extend_from_slice(&96_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&((frames.len() * 2) as u32).to_le_bytes());
        for frame in frames {
            wav.extend_from_slice(&frame.to_le_bytes());
        }
        fs::write(kit.join("tone60.wav"), wav).expect("写 WAV");

        let mut project = yeban_model::samples::filled_project();
        let lead = project
            .tracks
            .iter()
            .find(|(_, track)| track.name == "Lead")
            .map(|(id, _)| *id)
            .expect("filled_project 必须有 Lead 轨");
        let mut clip_ids = Vec::new();
        for placement in project
            .tracks
            .get_mut(&lead)
            .expect("Lead")
            .clips
            .values_mut()
        {
            placement.start_tick = 0;
            placement.duration_ticks = 960;
            placement.loop_config = yeban_model::LoopConfig::default();
            clip_ids.push(placement.clip_id);
        }
        for clip_id in clip_ids {
            if let Some(entry) = project.clip_pool.get_mut(&clip_id)
                && let ClipContent::Midi { notes } = &mut entry.content
            {
                notes.clear();
                let note_id = EntityId::new();
                notes.insert(note_id, MidiNote::new(note_id, 0, 60, 960));
            }
        }
        project.tracks.get_mut(&lead).expect("Lead").sampler = Some(yeban_model::SamplerBinding {
            note_octave_override: None,
            path: "assets/samples/kit/001.sfz".to_owned(),
        });
        project.validate().expect("夹具工程必须合法");

        let song = root.join("proj/song.yeban");
        let (archive, missing) =
            prepare_archive_for_save(project, &song, None).expect("保存前收集");
        assert_eq!(missing, 0, "夹具里的样本都在盘上，不应报缺失");
        assert!(
            archive
                .project
                .embedded_samples
                .contains_key("assets/samples/kit/001.sfz"),
            "SFZ 文本必须进 embedded_samples 表: {:?}",
            archive.project.embedded_samples
        );
        assert!(
            archive
                .project
                .embedded_samples
                .contains_key("assets/samples/kit/tone60.wav"),
            "当前音符触发的新样本必须进 embedded_samples 表: {:?}",
            archive.project.embedded_samples
        );
        assert!(
            archive.assets.len() >= 2,
            "SFZ 文本与 WAV 字节都必须进资产池: {:?}",
            archive.assets
        );
    }

    // ---- T-FIX-244：脏标记跟"模型已变更"走，不跟"引擎发布成功"走。 ----
    struct TFix244TempDir(PathBuf);
    impl Drop for TFix244TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 一条带音符、且未绑采样器的轨：给它绑新 SFZ 才能在发布时触发 Deferred。
    fn tfix244_pitched_project() -> (YebanProjectV1, EntityId) {
        let project = yeban_model::samples::filled_project();
        let track_id = project
            .tracks
            .iter()
            .find(|(_, track)| {
                track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
            })
            .map(|(id, _)| *id)
            .expect("filled_project 必须有带音符的轨");
        (project, track_id)
    }

    #[test]
    fn t_fix_244_deferred_publish_marks_dirty_and_blocks_quit() {
        let root = std::env::temp_dir().join(format!(
            "yeban-tui-tfix244-deferred-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _cleanup = TFix244TempDir(root.clone());
        std::fs::create_dir_all(&root).expect("建临时目录");
        let sfz = root.join("kit.sfz");
        std::fs::write(&sfz, "<region> sample=tone.wav key=60 pitch_keycenter=60\n")
            .expect("写 SFZ 夹具");

        let (project, track_id) = tfix244_pitched_project();
        let old_sampler = project.tracks[&track_id].sampler.clone();
        let mut app = App::with_project(project).expect("app");
        assert!(!app.dirty);

        app.commit_edit(
            "tui: bind uncached sampler",
            vec![Op::SetTrackSampler {
                track_id,
                old_sampler,
                new_sampler: Some(yeban_model::SamplerBinding {
                    note_octave_override: None,
                    path: sfz.to_string_lossy().into_owned(),
                }),
            }],
            "绑定音源",
        );
        assert!(app.publish_deferred, "首跳发布应因未缓存 SFZ 而延后");
        assert!(app.dirty, "模型已变更 ⇒ 必须置脏，发布延后也要置脏");

        app.execute_command("q");
        assert!(!app.quit, "有未保存修改时 :q 必须被拦");
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("有未保存修改")),
            "应提示先保存，got {:?}",
            app.log.last()
        );
    }

    #[test]
    fn t_fix_244_failed_publish_marks_dirty_and_blocks_quit() {
        let (project, track_id) = tfix244_pitched_project();
        let old_sampler = project.tracks[&track_id].sampler.clone();
        let mut app = App::with_project(project).expect("app");

        let missing = std::env::temp_dir().join(format!(
            "yeban-tui-tfix244-missing-{}-{}.sfz",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        assert!(!missing.exists());

        app.commit_edit(
            "tui: bind missing sampler",
            vec![Op::SetTrackSampler {
                track_id,
                old_sampler,
                new_sampler: Some(yeban_model::SamplerBinding {
                    note_octave_override: None,
                    path: missing.to_string_lossy().into_owned(),
                }),
            }],
            "绑定音源",
        );
        assert!(app.dirty, "发布失败也不能吞掉已提交编辑的脏标记");
        app.execute_command("q");
        assert!(!app.quit, "发布失败后 :q 同样必须被拦");
    }

    #[test]
    fn t_fix_244_save_clears_dirty_and_late_publish_success_does_not_redirty() {
        struct TempCleanup(PathBuf);
        impl Drop for TempCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = std::env::temp_dir().join(format!(
            "yeban-tui-tfix244-save-{}-{}.yeban",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _cleanup = TempCleanup(path.clone());

        let mut app = App::with_demo().expect("demo app");
        app.toggle_mute();
        assert!(app.dirty);
        app.save_to(&path).expect("保存成功");
        assert!(!app.dirty, "保存成功后必须清脏");

        // 模拟一笔早先延后的发布在保存之后才成功：不得把 dirty 置回。
        app.publish_deferred = true;
        app.retry_deferred_publish();
        assert!(!app.publish_deferred, "延后标记应被清掉");
        assert!(!app.dirty, "保存后延迟发布成功不得把 dirty 置回 true");
    }
}
