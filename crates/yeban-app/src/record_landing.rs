//! ⭐ **录音落点**：把停录后**已经落盘**的那段 WAV 变成工程里的**真实编辑**
//! （`Op::AddClip`）＋ 容器资产池里的**字节**（`assets/{sha256}`）。
//!
//! 任务单 `#132`，依据 `#130` 的分层分析里的 **L2 ＋ L3**（⛔ 不做 L1／L4／L5）。
//!
//! ## 它补的是哪一格（两条缺口，一条数据流）
//!
//! `#130` 之后，"按录音键 ⇒ 真设备的样本逐码落盘"已经成立
//! （`crate::record_device` ＋ `crate::record_wav` 的判据）。但那段 WAV 到此为止
//! **什么都不是**：工程的 `clip_pool` 里没有它，容器的 `assets/{sha256}` 里也没有它
//! ⇒ 人录完音，工程**一位没变**。本模块把最后这一跳接上：
//!
//! ```text
//! 停录成功（frames > 0）＋ 磁盘上那段 WAV
//!   ──▶ plan_recording（**纯函数**：读字节 ＋ 解码校验 ＋ 内容哈希 ＋ 派生 `Op::AddClip`）
//!   ──▶ land_recording（薄：读端口里的工程 → 规划 → **一次**提交 → 登记资产池）
//!   ──▶ UndoPort 的 CAS 池（`assets/{sha256}` 的字节）
//!   ──▶ host::wire_save 把池写进容器（`ProjectArchive::assets`）
//! ```
//!
//! ## 为什么把"WAV 路径 → 资产 ＋ clip"抽成纯函数
//!
//! 这是本单的**设计要点**（与 `#129` 的 `new_project_archive` 同款）：判据要能在
//! **无桌面会话**下判 ① 资产哈希是不是字节的 SHA-256、② clip 指向的资产对不对得上 ——
//! 那两句话都不需要窗口、不需要设备、不需要工程会话。于是"从 WAV 到 `(资产, clip)`"
//! 的唯一实现是 [`plan_recording`]（只读文件、⛔ 不碰会话、⛔ 不写盘），
//! 而"把它变成一次提交"是 [`land_recording`]（薄到只剩端口调用）。
//!
//! ## ⚠ 三条**如实登记**（⛔ 不假装）
//!
//! 1. ⭐ **L4（素材池面板）未做**：全仓 `.slint` 对 `clip_pool` 的投影是 **0 处**
//!    （`crate::bridge` 只用 `clip_pool` 取 MIDI 音符，`bridge.rs:1600`）⇒ 本片之后，
//!    用户在界面上**看得到的仍然只有**状态行／`stderr`：那条 clip **不会**出现在任何
//!    素材池面板里。⛔ 本单不碰 `.slint`（那会动绘制预算，属 L1／L4）。
//! 2. ⭐ **L5（frames → tick 摆放）未做**：本片只在 `clip_pool` **登记**片段
//!    （`Op::AddClip`），⛔ 不写 `Op::AddClipPlacement` ⇒ 那段音频**不会**自动出现在
//!    卷帘的某条轨道上。摆放需要"目的地策略 ＋ 帧 → tick（向上取整）"的换算，而
//!    后者的**唯一实现**在 `crates/yeban-mcp/src/domain/render_math.rs`
//!    （`yeban-mcp` 是默认关闭的可选依赖，app 够不着）⇒ 在这里照抄会造出**第二份**
//!    tick 换算。与 `crate::import_audio` 模块文档第 2 条是**同一条**边界。
//! 3. **撤销不回收资产池里的字节**：资产字节不是 `Op` 的载荷（模型的 `Op` 全集没有
//!    资产变体）⇒ `AddClip` 的逆只移除 `clip_pool` 里的条目，⛔ 不会删掉容器的
//!    `assets/{sha256}`。与 `crate::import_audio` 模块文档第 3 条**逐字同款**。
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

use yeban_model::{AssetHash, ClipPoolEntry, EntityId, Op, OpOrigin, YebanProjectV1};

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

/// 一次真的落进工程的录音（[`land_recording`] 的结果）。
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
    /// `true` = 本次真的提交了 `Op::AddClip`；`false` = 池里已有同内容条目。
    pub committed: bool,
    /// 提交身份（`None` = 没有提交，即幂等命中）。
    pub commit: Option<EntityId>,
}

impl RecordedClip {
    /// 一行可读摘要（产品路径打 `stderr`；资产哈希就是容器里的 `assets/{sha256}`）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "recorded-clip: path={} name={} asset={} asset-bytes={} frames={} rate={} \
             channels={} bits={} float={} clip={} committed={} commit={}",
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
    /// 提交被会话拒绝（工程与游标一位不动）。
    Refused(String),
}

impl std::fmt::Display for RecordLandingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProject => f.write_str("撤销端口里此刻没有工程，录音无处可落"),
            Self::Plan(error) => write!(f, "录音产物无法变成工程资产: {error}"),
            Self::Refused(reason) => write!(f, "录音进工程的提交被拒绝: {reason}"),
        }
    }
}

impl std::error::Error for RecordLandingError {}

impl From<AudioImportError> for RecordLandingError {
    fn from(error: AudioImportError) -> Self {
        Self::Plan(error)
    }
}

/// ⭐ **停录成功之后的唯一落地函数**（产品路径 `host::wire_record` 调它）。
///
/// 顺序是承重的：**先**规划（读字节 ＋ 解码校验 ＋ 哈希），**再**提交一次
/// `Op::AddClip`，**最后**才登记资产池 —— 提交被拒时池里不会留下"没有任何片段引用"
/// 的孤儿字节；幂等命中时（`ops` 为空）一位都不提交，只确保字节在池里。
///
/// `now_ms` 是提交时间戳（调用方给；判据给固定值 ⇒ 可复现）。
///
/// # Errors
///
/// [`RecordLandingError::NoProject`] / [`RecordLandingError::Plan`] /
/// [`RecordLandingError::Refused`]。任何一种都意味着**工程一位没动**（或只有池没变）。
pub fn land_recording(
    port: &crate::undo::UndoPort,
    path: &Path,
    now_ms: u64,
) -> Result<RecordedClip, RecordLandingError> {
    let project = port.try_project().ok_or(RecordLandingError::NoProject)?;
    let plan = plan_recording(path, &project)?;
    let commit = if plan.ops.is_empty() {
        None
    } else {
        // 来源 = `UserUi`：这是"人按了录音键"的直接结果（见模块文档的登记）。
        let commit = port
            .commit_ops_with_origin(now_ms, OpOrigin::UserUi, "record audio clip", plan.ops)
            .map_err(|refusal| RecordLandingError::Refused(refusal.to_string()))?;
        Some(commit)
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
        committed: commit.is_some(),
        commit,
    })
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
}
