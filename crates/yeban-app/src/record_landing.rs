//! ⭐ **录音落点**：把停录后**已经落盘**的那段 WAV 变成工程里的**真实编辑**
//! （`Op::AddClip` ＋ `Op::AddClipPlacement`）＋ 容器资产池里的**字节**
//! （`assets/{sha256}`）。
//!
//! 任务单 `#132`（L2＋L3：进池）与 `#134`（L5：摆到轨道上、能响）；
//! 依据 `#130` 的分层分析。L1／L4 不归本模块（L4 已由 `#133` 接力）。
//!
//! ## 它补的是哪一格（三条缺口，一条数据流）
//!
//! `#130` 之后，"按录音键 ⇒ 真设备的样本逐码落盘"已经成立
//! （`crate::record_device` ＋ `crate::record_wav` 的判据）。但那段 WAV 到此为止
//! **什么都不是**：工程的 `clip_pool` 里没有它，容器的 `assets/{sha256}` 里也没有它，
//! 更不在任何轨道上 ⇒ 人录完音，工程**一位没变**、播放**没有声音**。本模块把这几跳
//! 接上：
//!
//! ```text
//! 停录成功（frames > 0）＋ 磁盘上那段 WAV
//!   ──▶ plan_recording（**纯函数**：读字节 ＋ 解码校验 ＋ 内容哈希 ＋ 派生 `Op::AddClip`）
//!   ──▶ land_recording_at（薄：读工程 → 规划 → 算摆放 → **一次**提交 → 登记资产池）
//!         ├─ 目标轨 = 选中的／待录的音频轨（T-FIX-239：⛔ 不再默认第一条音频轨）
//!         ├─ 起点   = 起录时捕获的 tick（`RecordReport::start_tick`，⛔ 不是停录播放头）
//!         └─ 长度   = frames_to_ticks_ceil(frames, PPQ, bpm, facts.sample_rate)
//!   ──▶ UndoPort 的 CAS 池（`assets/{sha256}` 的字节）
//!   ──▶ host::wire_save 把池写进容器（`ProjectArchive.assets`）
//! ```
//!
//! ## 为什么把"WAV 路径 → 资产 ＋ clip"抽成纯函数
//!
//! 这是 `#132` 的**设计要点**（与 `#129` 的 `new_project_archive` 同款）：判据要能在
//! **无桌面会话**下判 ① 资产哈希是不是字节的 SHA-256、② clip 指向的资产对不对得上 ——
//! 那两句话都不需要窗口、不需要设备、不需要工程会话。于是"从 WAV 到 `(资产, clip)`"
//! 的唯一实现是 [`plan_recording`]（只读文件、⛔ 不碰会话、⛔ 不写盘），
//! 而"把它变成一次提交"是 [`land_recording_at`]（薄到只剩端口调用）。
//! ⭐ 摆放的算术（帧 → tick）在**模型层** [`yeban_model::time_math::frames_to_ticks_ceil`]，
//! 这里只决定"哪条轨、哪个起点"。
//!
//! ## 三条**如实登记**（⛔ 不假装）
//!
//! 1. ⭐ **L4（素材池面板）**：`#132` 时 `.slint` 对 `clip_pool` 的投影是 0 处；
//!    该格已由 `#133` 落地（`ViewState::asset_pool` → `asset-pool-*` 三个数组）。
//!    本模块**仍然不碰** `.slint`／`apply_view`（绘制预算一位不动）。
//! 2. ⭐ **L5（frames → tick 摆放）已落地（任务单 `#134`）**：本模块除了 `Op::AddClip`
//!    ＋ 资产，还在**同一次提交**里写一条 `Op::AddClipPlacement` ⇒ 那段音频真的落在
//!    **选中的／待录的音频轨**的**起录 tick** 上（T-FIX-239：起点是起录时捕获的
//!    `RecordReport::start_tick`，⛔ 不是停录时播放头在哪；目标轨是起录时解析的
//!    [`RecordTarget`]，⛔ 不再默认第一条音频轨）。帧 → tick 的换算已下移到
//!    [`yeban_model::time_math::frames_to_ticks_ceil`]（模型层纯函数，app 直接调用，
//!    ⛔ 不再"够不着 mcp"）；`yeban-mcp` 那份既有实现**保留**并登记为重复实现
//!    （见模型层模块文档：那条文件被裸 `rustc` 验证脚手架 `#[path]` 引入，
//!    改成跨 crate 调用会断掉本机零依赖验证路径）。
//! 3. **撤销不回收资产池里的字节**：资产字节不是 `Op` 的载荷（模型的 `Op` 全集没有
//!    资产变体）⇒ `AddClip` 的逆只移除 `clip_pool` 里的条目，⛔ 不会删掉容器的
//!    `assets/{sha256}`。与 `crate::import_audio` 模块文档第 3 条**逐字同款**。
//!
//! ⚠ **听得到的边界（如实）**：实时引擎快照**不消费** `ClipContent::Audio`
//! （`yeban-engine/src/snapshot.rs` 的 `project_schedules` 第 5 条：音频不产生声音）⇒
//! 走带（`EngineHost`）**听不到**这段录音。**离线渲染**入口
//! （`crate::render_core::render_project_f32_with_assets`，`--export-wav` 与录音侧共用）
//! 会真的把它混进母带 ⇒ 本单的"能响"判据走那条真实渲染路径。实时那半属引擎层，本单
//! ⛔ 不改 `crates/yeban-engine/**`。
//!
//! ## 与"音频导入"的关系（⛔ 没有第二份解码／哈希／幂等实现）
//!
//! 规划一律经过 `crate::import_audio::plan_audio_import`：同一份 WAV 读取器
//! （`yeban_render::wav::read_plain_wav`）、同一个内容哈希（`AssetHash::of_bytes`）、
//! 同一条幂等规则（`clip_pool` 里已有同内容且增益为 `0.0` 的音频条目 ⇒ 复用、⛔ 不发提交）。
//! 本模块**只**负责"录音"这一侧的两件不同的事：
//!
//! - **来源标记**：录音用 [`OpOrigin::UserUi`]（人按了录音键），⛔ 不是
//!   [`OpOrigin::Import`]（"外部工程／格式导入"描述的不是这件事）。
//!   `OpOrigin` 的变体全集在 `yeban-model` 且被 `schemas/ops.schema.json` 的
//!   `origin.oneOf` 钉住 ⇒ ⛔ 本单**不新增**变体（`AutomationRecord` 只描述自动化录制）。
//! - **资产池**：录音的字节进 [`crate::undo::UndoPort`] 的会话 CAS 池
//!   （导入那边由**调用方**——CLI 的 `register_audio_asset`／MCP 的响应 `notes`——负责）。

use std::path::{Path, PathBuf};

use yeban_model::{
    AssetHash, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, Op, OpOrigin, PPQ, TrackKind,
    YebanProjectV1, frames_to_ticks_ceil,
};

use crate::import_audio::{
    AudioImportError, AudioImportFacts, AudioImportOptions, plan_audio_import,
};

/// 一次"停录产物 → 资产 ＋ clip"的规划（[`plan_recording`] 的全部产物）。
///
/// 它是**纯数据**：拿它不需要工程会话，因此判据可以只喂一个 WAV 路径就把
/// ① `asset` 与 ② `clip` 两件事都读出来（无桌面会话）。
#[derive(Clone, Debug, PartialEq)]
pub struct RecordingPlan {
    /// 停录落盘的那段 WAV（事实来源；字节是从**这个路径**读回来的）。
    pub path: PathBuf,
    /// 内容寻址键 = WAV **完整字节**的 SHA-256（⛔ 不接受调用方声明）。
    pub asset: AssetHash,
    /// 要落进容器资产池的字节（= `std::fs::read(path)`，与 `asset` 一一对应）。
    pub bytes: Vec<u8>,
    /// 解码事实（帧数／采样率／声道／位深）。
    pub facts: AudioImportFacts,
    /// 要（或已经）在 `clip_pool` 里的条目。
    pub clip: ClipPoolEntry,
    /// 待提交的操作：空 = `clip_pool` 里已有同内容条目（幂等命中，一位都不改）。
    pub ops: Vec<Op>,
}

impl RecordingPlan {
    /// 这条规划会**新建**一条 `clip_pool` 条目吗。
    #[must_use]
    pub fn creates_clip(&self) -> bool {
        !self.ops.is_empty()
    }

    /// 一行可读摘要（产品路径用它打 `stderr`；键值对以空白分隔）。
    ///
    /// ⚠ 与 `imported-audio:` 同一条口径：值**不加引号**（路径／名字里有空白时这一行
    /// 不再可机械切分 —— 需要精确读数的调用方走结构化的字段，⛔ 不解析这一行）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "recorded-clip: path={} name={} asset={} asset-bytes={} frames={} rate={} \
             channels={} bits={} float={} clip={} creates-clip={}",
            self.path.display(),
            self.clip.name,
            self.asset,
            self.bytes.len(),
            self.facts.frames,
            self.facts.sample_rate,
            self.facts.channels,
            self.facts.bits_per_sample,
            self.facts.is_float,
            self.clip.id.to_canonical_string(),
            self.creates_clip(),
        )
    }
}

/// ⭐⭐ **纯函数**：`WAV 路径 → (资产, clip)`（⛔ 不碰会话、⛔ 不写盘）。
///
/// 只读 `path` 的字节，然后：
///
/// 1. 用**既有**的导入规划器解码校验一遍（⛔ 不登记一个渲染时才发现是坏的资产）；
/// 2. 内容哈希由**字节**算出（`AssetHash::of_bytes`）；
/// 3. 按同一条幂等规则派生 `Op::AddClip`（同内容条目已在 `clip_pool` ⇒ 空 `ops`）。
///
/// `project` 只用于第 3 步的幂等判断（⛔ 不被本函数修改）。
///
/// # Errors
///
/// 透传 [`AudioImportError`]：读失败／不是普通文件／超上限／推不出名字／解码失败。
pub fn plan_recording(
    path: &Path,
    project: &YebanProjectV1,
) -> Result<RecordingPlan, AudioImportError> {
    let plan = plan_audio_import(path, project, &AudioImportOptions::default())?;
    Ok(RecordingPlan {
        path: path.to_path_buf(),
        asset: plan.asset,
        bytes: plan.bytes,
        facts: plan.facts,
        clip: plan.clip,
        ops: plan.ops,
    })
}

/// 一条已经摆到轨道上的录音（[`land_recording_at`] 的产物，任务单 `#134`）。
///
/// 它逐字携带**那次提交**里的 `Op::AddClipPlacement` 载荷 ⇒ 判据能直接读出
/// 轨道 id ／ 起点 tick ／ 长度 tick ／ 摆放 id 与 clip id 的对应关系。
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedRecording {
    /// 目标音轨（T-FIX-239：起录时解析的 [`RecordTarget`]——选中的／待录的
    /// 音频轨；工程只有一条音频轨时就是它）。
    pub track_id: EntityId,
    /// 提交进 `TrackV3::clips` 的那条摆放（`id`／`clip_id`／`start_tick`／
    /// `duration_ticks`／`loop_config`／`muted` 逐字段可读）。
    pub placement: ClipPlacement,
}

/// 一次真的落进工程的录音（[`land_recording`]／[`land_recording_at`] 的结果）。
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedClip {
    /// 停录落盘的 WAV。
    pub path: PathBuf,
    /// 内容寻址键（= 那段 WAV 字节的 SHA-256）。
    pub asset: AssetHash,
    /// 已登记进会话资产池的字节数。
    pub asset_bytes: usize,
    /// `clip_pool` 里的条目身份（新建或幂等复用的那一条）。
    pub clip_id: EntityId,
    /// 片段显示名。
    pub clip_name: String,
    /// 解码事实。
    pub facts: AudioImportFacts,
    /// 本次的摆放（`None` = 工程里**没有**音频轨，无处可摆；如实登记，⛔ 不假装）。
    pub placement: Option<PlacedRecording>,
    /// `true` = 本次真的提交了操作（`AddClip` 和／或摆放）；`false` = 一位都没改。
    pub committed: bool,
    /// 提交身份（`None` = 没有提交，即幂等命中）。
    pub commit: Option<EntityId>,
}

impl RecordedClip {
    /// 一行可读摘要（产品路径打 `stderr`；资产哈希就是容器里的 `assets/{sha256}`）。
    #[must_use]
    pub fn summary(&self) -> String {
        let (placement_id, track_id, start_tick, duration_ticks) =
            self.placement.as_ref().map_or_else(
                || ("<none>".to_owned(), "<none>".to_owned(), 0, 0),
                |placed| {
                    (
                        placed.placement.id.to_canonical_string(),
                        placed.track_id.to_canonical_string(),
                        placed.placement.start_tick,
                        placed.placement.duration_ticks,
                    )
                },
            );
        format!(
            "recorded-clip: path={} name={} asset={} asset-bytes={} frames={} rate={} \
             channels={} bits={} float={} clip={} placement={placement_id} track={track_id} \
             start-tick={start_tick} duration-ticks={duration_ticks} committed={} commit={}",
            self.path.display(),
            self.clip_name,
            self.asset,
            self.asset_bytes,
            self.facts.frames,
            self.facts.sample_rate,
            self.facts.channels,
            self.facts.bits_per_sample,
            self.facts.is_float,
            self.clip_id.to_canonical_string(),
            self.committed,
            self.commit
                .map_or_else(|| "<none>".to_owned(), |id| id.to_canonical_string()),
        )
    }
}

/// 录音落点失败的原因（⛔ 不吞、⛔ 不猜）。
///
/// 为什么**不**派生 `PartialEq` / `Clone`：载荷里的 `AudioImportError` 带
/// `std::io::Error`（既不可比较也不可克隆）⇒ 比较／复制错误值没有意义；
/// 判据读的是 `Display` 文本与工程读数（`clip_pool` / 资产池）。
#[derive(Debug)]
pub enum RecordLandingError {
    /// 端口里此刻没有工程（控制面可以关掉工程）。
    NoProject,
    /// 读不到／解不开那段 WAV。
    Plan(AudioImportError),
    /// 停录产物**一帧都没有** ⇒ 既不登记片段、也不摆放（任务单 `#134` 的 §4 第 5 条）。
    ///
    /// 产品路径（`host::wire_record`）在调用本模块**之前**就按 `report.frames > 0`
    /// 挡了一层；本错误把同一条边界钉在落点函数**自己**身上 —— 任何直接调用
    /// `land_recording_at` 的路径（判据／未来的其它入口）都不会把 0 帧写成一条
    /// "播放时什么都没有"的摆放。
    EmptyRecording {
        /// 那段 0 帧 WAV 的路径。
        path: PathBuf,
    },
    /// 帧 → tick 换算没有答案（工程 BPM 非有限／≤ 0，或素材采样率为 0）。
    ///
    /// 模型层 [`frames_to_ticks_ceil`] 对病态输入返回 `None`（⛔ 没有哨兵值：
    /// 0 会被 `ClipPlacement::validate` 拒绝）⇒ 这里必须在**提交之前**明确拒绝，
    /// ⛔ 不许写一条时长错误的摆放。
    Ticks {
        /// 工程速度。
        bpm: f64,
        /// 素材采样率（Hz）。
        sample_rate: u32,
        /// 素材帧数。
        frames: u64,
    },
    /// 提交被会话拒绝（工程与游标一位不动）。
    Refused(String),
    /// ⭐ 多条音频轨，但没有一条被选中／待录（T-FIX-239）⇒ **响亮地拒绝**，
    /// ⛔ 不静默落到第一条音频轨。WAV 已落盘（⛔ 不丢），报告会说清原因。
    NoTargetTrack,
    /// ⭐ 指定的目标轨不存在、不是音频轨、或就是主总线（T-FIX-239）。
    TargetTrack {
        /// 调用方给的那个轨身份。
        track: EntityId,
    },
}

impl std::fmt::Display for RecordLandingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProject => f.write_str("撤销端口里此刻没有工程，录音无处可落"),
            Self::Plan(error) => write!(f, "录音产物无法变成工程资产: {error}"),
            Self::EmptyRecording { path } => write!(
                f,
                "`{}` 是 0 帧录音 ⇒ ⛔ 不登记片段、⛔ 不摆放",
                path.display()
            ),
            Self::Ticks {
                bpm,
                sample_rate,
                frames,
            } => write!(
                f,
                "无法把 {frames} 帧换算成 tick（bpm={bpm}, sample_rate={sample_rate}）\
                 ⇒ ⛔ 不摆放（时长会错）"
            ),
            Self::Refused(reason) => write!(f, "录音进工程的提交被拒绝: {reason}"),
            Self::NoTargetTrack => f.write_str(
                "工程里有多条音频轨，但没有一条被选中／待录 ⇒ 录音已落盘但没有进工程，\
                 ⛔ 不静默落到第一条音频轨；选中或待录一条音频轨后再录",
            ),
            Self::TargetTrack { track } => write!(
                f,
                "指定的目标轨 `{}` 不存在、不是音频轨、或就是主总线 ⇒ ⛔ 不摆放",
                track.to_canonical_string()
            ),
        }
    }
}

impl std::error::Error for RecordLandingError {}

impl From<AudioImportError> for RecordLandingError {
    fn from(error: AudioImportError) -> Self {
        Self::Plan(error)
    }
}

/// ⭐ **录音落到哪条轨**（T-FIX-239）：起录时由调用方解析一次、停录时原样用。
///
/// ⛔ **不再**有"默认第一条音频轨"：多条音频轨时没有选择 ⇒ 响亮地拒绝
/// （[`RecordLandingError::NoTargetTrack`]），⛔ 不猜。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordTarget {
    /// 落到这条音频轨。
    Track(EntityId),
    /// 工程里**没有**音频轨 ⇒ 软处理：片段照常登记进 `clip_pool`＋资产池，
    /// 摆放为 `None`（旧口径，见 [`plan_placement`]）。
    NoAudioTrack,
    /// 工程里有多条音频轨，但没有一条被选中／待录 ⇒ 响亮地拒绝。
    NoSelection,
}

/// 工程里全部音频轨的身份（`BTreeMap` 键序 ⇒ 身份升序，跨进程确定）。
///
/// 主总线按构造是 `TrackKind::Master`，不会被选中；这里仍显式排除一次，避免未来
/// 有工程把主总线误标成 Audio 时把素材摆到唯一的声学出口上。
#[must_use]
pub fn audio_track_ids(project: &YebanProjectV1) -> Vec<EntityId> {
    project
        .tracks
        .iter()
        .filter(|(id, track)| **id != project.master_bus_track_id && track.kind == TrackKind::Audio)
        .map(|(id, _)| *id)
        .collect()
}

/// ⭐ **纯函数**：从"待录的轨 ＋ 选中的轨"解析出本次录音的目标轨（T-FIX-239）。
///
/// 优先级（**第一条**命中的胜出）：
/// 1. **待录**的音频轨（`armed` 里按调用方给的顺序 —— 调用方传 UI 行序）；
/// 2. **选中**的音频轨（`selected`）；
/// 3. 工程里**恰好只有一条**音频轨 ⇒ 就是它（没得选 ⇒ 不是"静默猜"）；
/// 4. 否则 [`RecordTarget::NoSelection`]（响亮地拒绝，⛔ 不落到第一条）。
///
/// ⛔ **非音频轨（MIDI／Aux／Master）与不存在的 id 一律跳过**：把音频片段摆到
/// MIDI 轨上渲染时仍然是静音（`clip_pool` 里那条是 `ClipContent::Audio`），
/// 那是"看起来摆了、其实听不见"的假成功。
#[must_use]
pub fn resolve_record_target(
    project: &YebanProjectV1,
    armed: &[EntityId],
    selected: Option<EntityId>,
) -> RecordTarget {
    let audio = audio_track_ids(project);
    if audio.is_empty() {
        return RecordTarget::NoAudioTrack;
    }
    if let Some(track) = armed.iter().find(|id| audio.contains(id)) {
        return RecordTarget::Track(*track);
    }
    if let Some(track) = selected.filter(|id| audio.contains(id)) {
        return RecordTarget::Track(track);
    }
    if audio.len() == 1 {
        return RecordTarget::Track(audio[0]);
    }
    RecordTarget::NoSelection
}

/// ⭐ **停录成功之后的落地函数**（播放头取 0、无目标轨的兼容形态）。
///
/// 它转调 [`land_recording_at`]，起点 tick 取 `0`、目标轨取
/// [`RecordTarget::NoAudioTrack`]（片段照常登记，⛔ 不摆放）。
/// 产品路径走后者；保留本函数是为了**不打断**既有调用方与判据的签名。
/// （本函数今天没有调用方；见 T-FIX-239 的收据。）
///
/// # Errors
///
/// 与 [`land_recording_at`] 相同。
pub fn land_recording(
    port: &crate::undo::UndoPort,
    path: &Path,
    now_ms: u64,
) -> Result<RecordedClip, RecordLandingError> {
    land_recording_at(port, path, now_ms, 0, RecordTarget::NoAudioTrack)
}

/// ⭐⭐ **停录成功之后的唯一落地函数**（任务单 `#132` 的 L2＋L3，`#134` 补上 L5）。
///
/// 顺序是承重的：
///
/// 1. **先**规划（读字节 ＋ 解码校验 ＋ 哈希）；
/// 2. **再**算摆放（起录时解析的 [`RecordTarget`] ＋ `start_tick` 起点 ＋ 帧 → tick 长度）；
/// 3. **一次**提交 `Op::AddClip`（若片段是新的）＋ `Op::AddClipPlacement`；
/// 4. **最后**才登记资产池 —— 提交被拒时池里不会留下"没有任何片段引用"的孤儿字节。
///
/// ⚠ 两条操作在**同一个** `Vec<Op>` 里 ⇒ 同一次提交、同一份撤销单元：`AddClip`
/// 的片段**先**进池、`AddClipPlacement` 的摆放**后**引用它（顺序即施加顺序）。
///
/// `start_tick` 是摆放起点（T-FIX-239：调用方在起录时捕获的
/// `起录播放头 + 预备拍 tick`，见 [`RecordReport::start_tick`]；
/// ⛔ **不再**是停录时播放头在哪）。`target` 是起录时解析的目标轨
/// （[`resolve_record_target`]）；多条音频轨且没有选择 ⇒
/// [`RecordLandingError::NoTargetTrack`]（响亮地拒绝，⛔ 不静默落到第一条）。
/// **0 帧** ⇒ [`RecordLandingError::EmptyRecording`]，工程一位不动（⛔ 不 AddClip、
/// ⛔ 不摆放）。
///
/// `now_ms` 是提交时间戳（调用方给；判据给固定值 ⇒ 可复现）。
///
/// # Errors
///
/// [`RecordLandingError::NoProject`] / [`RecordLandingError::Plan`] /
/// [`RecordLandingError::EmptyRecording`] / [`RecordLandingError::Ticks`] /
/// [`RecordLandingError::NoTargetTrack`] / [`RecordLandingError::TargetTrack`] /
/// [`RecordLandingError::Refused`]。任何一种都意味着**工程一位没动**（或只有池没变）。
pub fn land_recording_at(
    port: &crate::undo::UndoPort,
    path: &Path,
    now_ms: u64,
    start_tick: u64,
    target: RecordTarget,
) -> Result<RecordedClip, RecordLandingError> {
    let project = port.try_project().ok_or(RecordLandingError::NoProject)?;
    let plan = plan_recording(path, &project)?;
    if plan.facts.frames == 0 {
        return Err(RecordLandingError::EmptyRecording { path: plan.path });
    }
    let placement = plan_placement(&project, &plan, start_tick, target)?;
    let mut ops = plan.ops;
    if let Some(placed) = &placement {
        ops.push(Op::AddClipPlacement {
            track_id: placed.track_id,
            placement: placed.placement,
        });
    }
    let commit = if ops.is_empty() {
        None
    } else {
        // 来源 = `UserUi`：这是"人按了录音键"的直接结果（见模块文档的登记）。
        Some(
            port.commit_ops_with_origin(now_ms, OpOrigin::UserUi, "record audio clip", ops)
                .map_err(|refusal| RecordLandingError::Refused(refusal.to_string()))?,
        )
    };
    // 资产池登记（幂等；同键不重复）。放在提交**之后**，理由见函数头。
    port.insert_asset(plan.asset.clone(), plan.bytes.clone());
    Ok(RecordedClip {
        path: plan.path,
        asset: plan.asset,
        asset_bytes: plan.bytes.len(),
        clip_id: plan.clip.id,
        clip_name: plan.clip.name,
        facts: plan.facts,
        placement,
        committed: commit.is_some(),
        commit,
    })
}

/// 摆放的**目的地策略**（任务单 `#134`；T-FIX-239 重写了"哪条轨"与"起点"）。
///
/// - **哪条轨**：调用方起录时解析的 [`RecordTarget`]（见 [`resolve_record_target`]）。
///   `Track(id)` ⇒ 校验它存在＋是音频轨＋不是主总线，否则
///   [`RecordLandingError::TargetTrack`]；`NoAudioTrack` ⇒ `Ok(None)`
///   （工程里没有音频轨，无处可摆；如实登记，⛔ 不硬塞到别的轨）；
///   `NoSelection` ⇒ [`RecordLandingError::NoTargetTrack`]（响亮地拒绝，
///   ⛔ 不静默落到第一条音频轨）。
///   ⛔ 不挑 MIDI／Aux／Master：把音频片段摆到 MIDI 轨上渲染时仍然是静音
///   （`clip_pool` 里那条是 `ClipContent::Audio`），那是"看起来摆了、
///   其实听不见"的假成功。
/// - **起点**：调用方给的 `start_tick`（T-FIX-239：起录时捕获的
///   `起录播放头 + 预备拍 tick`；⛔ 不是停录时播放头在哪）。
/// - **长度**：`frames_to_ticks_ceil(frames, PPQ, project.bpm, facts.sample_rate)`。
///   用**素材自己的**采样率（`facts.sample_rate`）—— `frames` 是它数出来的帧；
///   产品路径上它与工程采样率同为 48 kHz（`RecordOptions::default().sample_rate`，
///   工程缺省 48 kHz）。⚠ 票面写"用工程采样率"：两者同值时读数逐字相同；
///   若未来两者不同，用工程率会把时长算成另一个秒数（同一口径见 MCP 的
///   `import_audio.rs`，它同样传 `facts.sample_rate`）。
///
/// **没有音频轨** ⇒ `Ok(None)`：工程里无处可摆（如实登记，⛔ 不硬塞到别的轨）。
fn plan_placement(
    project: &YebanProjectV1,
    plan: &RecordingPlan,
    start_tick: u64,
    target: RecordTarget,
) -> Result<Option<PlacedRecording>, RecordLandingError> {
    let track_id = match target {
        RecordTarget::Track(id) => {
            let is_audio = project
                .tracks
                .get(&id)
                .is_some_and(|track| track.kind == TrackKind::Audio);
            if !is_audio || id == project.master_bus_track_id {
                return Err(RecordLandingError::TargetTrack { track: id });
            }
            id
        }
        RecordTarget::NoAudioTrack => return Ok(None),
        RecordTarget::NoSelection => return Err(RecordLandingError::NoTargetTrack),
    };
    let duration_ticks =
        frames_to_ticks_ceil(plan.facts.frames, PPQ, project.bpm, plan.facts.sample_rate).ok_or(
            RecordLandingError::Ticks {
                bpm: project.bpm,
                sample_rate: plan.facts.sample_rate,
                frames: plan.facts.frames,
            },
        )?;
    Ok(Some(PlacedRecording {
        track_id,
        placement: ClipPlacement {
            id: EntityId::new(),
            clip_id: plan.clip.id,
            start_tick,
            duration_ticks,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写一段**最小合法**的 16-bit 单声道 WAV（判据自己的夹具 ⇒ ⛔ 不依赖设备）。
    fn write_wav(path: &Path, samples: &[i16]) {
        let data_len = u32::try_from(samples.len() * 2).expect("夹具长度");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&96_000u32.to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes).expect("写夹具 WAV");
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-record-landing-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// ⭐ 纯函数的两句话：资产哈希 = 字节哈希；clip 的 `ClipContent::Audio` 指向它。
    #[test]
    fn the_plan_hashes_the_wav_bytes_and_points_the_clip_at_them() {
        let dir = scratch_dir();
        let path = dir.join("take.wav");
        write_wav(&path, &[0, 1_000, -1_000, 32_000]);
        let project = yeban_model::samples::default_project();

        let plan = plan_recording(&path, &project).expect("规划");
        let bytes = std::fs::read(&path).expect("读回 WAV");
        assert_eq!(
            plan.asset,
            AssetHash::of_bytes(&bytes),
            "哈希必须由字节算出"
        );
        assert_eq!(plan.bytes, bytes, "落池的字节必须与磁盘逐字相同");
        assert!(plan.creates_clip(), "空工程里第一次录音必须新建片段");
        let yeban_model::ClipContent::Audio { asset, gain_db } = &plan.clip.content else {
            panic!("录音片段必须是 Audio 内容");
        };
        assert_eq!(*asset, plan.asset, "clip 必须指向同一个 sha256");
        assert_eq!(*gain_db, 0.0, "单位增益");
        assert!(
            plan.facts.frames == 4 && plan.facts.channels == 1,
            "解码事实必须如实：{plan:?}"
        );

        // 幂等：把这条 clip 放进工程再规划一次 ⇒ 空 `ops`，一位都不改。
        let mut with_clip = project;
        with_clip.clip_pool.insert(plan.clip.id, plan.clip.clone());
        let again = plan_recording(&path, &with_clip).expect("二次规划");
        assert!(!again.creates_clip(), "同内容必须幂等命中");
        assert_eq!(again.clip.id, plan.clip.id, "复用同一条片段");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 两条音频轨的夹具工程：`filled_project` 的 Bass ＋ 新加一条音频轨。
    fn two_audio_tracks_project() -> (YebanProjectV1, EntityId, EntityId) {
        let mut project = yeban_model::samples::filled_project();
        let first = audio_track_ids(&project)[0];
        let second = EntityId::new();
        project.tracks.insert(
            second,
            yeban_model::TrackV3 {
                id: second,
                name: "Vox".to_owned(),
                kind: TrackKind::Audio,
                ..yeban_model::TrackV3::default()
            },
        );
        (project, first, second)
    }

    /// ⭐ **T-FIX-239**：待录的音频轨优先于选中的。
    #[test]
    fn armed_audio_track_wins_over_selected() {
        let (project, first, second) = two_audio_tracks_project();
        assert_eq!(
            resolve_record_target(&project, &[second], Some(first)),
            RecordTarget::Track(second),
            "⭐ 待录轨必须赢过选中轨"
        );
    }

    /// ⭐ **T-FIX-239**：没有待录时，选中的音频轨是目标。
    #[test]
    fn selected_audio_track_wins_when_nothing_armed() {
        let (project, first, second) = two_audio_tracks_project();
        assert_eq!(
            resolve_record_target(&project, &[], Some(second)),
            RecordTarget::Track(second),
            "⭐ 选中第二条音频轨 ⇒ 必须落到第二条（⛔ 不是键序第一条）"
        );
        assert_eq!(
            resolve_record_target(&project, &[], Some(first)),
            RecordTarget::Track(first),
            "⭐ 选中第一条 ⇒ 落到第一条"
        );
    }

    /// ⭐ **T-FIX-239**：工程里恰好只有一条音频轨 ⇒ 就是它（没得选 ⇒ 不是"静默猜"）。
    #[test]
    fn single_audio_track_is_the_target_without_any_selection() {
        let project = yeban_model::samples::filled_project();
        let only = audio_track_ids(&project)[0];
        assert_eq!(
            resolve_record_target(&project, &[], None),
            RecordTarget::Track(only),
            "⭐ 单音频轨工程不选不待录 ⇒ 仍落到那条轨"
        );
    }

    /// ⭐ **T-FIX-239**：多条音频轨、既不选也不待录 ⇒ `NoSelection`
    ///（落点方响亮地拒绝，⛔ 不落到第一条）。
    #[test]
    fn multiple_audio_tracks_without_selection_is_no_selection() {
        let (project, _, _) = two_audio_tracks_project();
        assert_eq!(
            resolve_record_target(&project, &[], None),
            RecordTarget::NoSelection,
            "⭐ 两条音频轨、无选择 ⇒ 必须拒绝（⛔ 不许默认第一条）"
        );
    }

    /// ⭐ **T-FIX-239**：工程里没有音频轨 ⇒ `NoAudioTrack`（软处理：登记片段、不摆放）。
    #[test]
    fn no_audio_track_is_no_audio_track() {
        let project = yeban_model::samples::default_project();
        assert_eq!(
            resolve_record_target(&project, &[], None),
            RecordTarget::NoAudioTrack,
            "⭐ 只有主总线的工程 ⇒ 没有音频轨"
        );
    }

    /// ⭐ **T-FIX-239**：非音频轨与不存在的 id 一律跳过（⛔ 不摆到 MIDI 轨上假成功）。
    #[test]
    fn non_audio_and_stale_ids_are_skipped() {
        let (project, _, second) = two_audio_tracks_project();
        let midi = project
            .tracks
            .iter()
            .find(|(_, track)| track.kind == TrackKind::Midi)
            .map(|(id, _)| *id)
            .expect("夹具里有一条 MIDI 轨");
        let stale = EntityId::new();
        // 待录里只有 MIDI＋不存在的 id ⇒ 都跳过 ⇒ 没有选择。
        assert_eq!(
            resolve_record_target(&project, &[midi, stale], Some(midi)),
            RecordTarget::NoSelection,
            "⭐ 待录/选中里只有 MIDI 轨与不存在的 id ⇒ 必须拒绝"
        );
        // 待录里 MIDI 在前、音频在后 ⇒ 跳过 MIDI，落到音频轨。
        assert_eq!(
            resolve_record_target(&project, &[midi, second], None),
            RecordTarget::Track(second),
            "⭐ 待录里的 MIDI 轨必须跳过"
        );
        // 选中主总线 ⇒ 不是音频轨 ⇒ 跳过；单条音频轨时仍落到那条音频轨。
        let single = yeban_model::samples::filled_project();
        let only = audio_track_ids(&single)[0];
        assert_eq!(
            resolve_record_target(&single, &[], Some(single.master_bus_track_id)),
            RecordTarget::Track(only),
            "⭐ 主总线永远不是目标"
        );
    }
}
