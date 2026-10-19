//! 音频文件**导入**：把磁盘上的一份音频变成工程里的**真实编辑操作**（`Op::AddClip`），
//! 并把那份字节交给调用方落进容器的 CAS 资产池（`assets/{sha256}`）。
//!
//! 这是 **MCP 侧 `yeban_import_audio` 的人类入口**：那条工具已实现且有端到端判据
//! （`crates/yeban-mcp/src/tools.rs:984`），但 `docs/ledger/feature-alignment.md:105`
//! 记着 UI 侧「无…状态：待接线」⇒ 一个人此前**没有任何**办法把音频弄进工程。
//!
//! ## 为什么落在这里（⛔ 零新增依赖边）
//!
//! 解码用的是 `yeban-render` 的 **WAV 读取器**（[`yeban_render::wav::read_plain_wav`]），
//! 而 `yeban-app` **已经**依赖 `yeban-render`（`--sfz` / `--export-wav` 都在用）。
//! 那份读取器不是本模块新写的第二份解码器：
//!
//! - `docs/ledger/feature-alignment.md:146` 逐字记它是「归一化的**唯一一份实现**」，
//!   集成者的裁决是「`app` 与 `mcp` **都**依赖它 ⇒ 不需要新 crate、不需要第二份实现」；
//! - `crates/yeban-mcp/src/domain/sfz.rs:151`（MCP 侧）与
//!   `crates/yeban-app/src/sfz_load.rs:206`（app 侧，**生产路径**）已经在用它。
//!
//! ⇒ 本切片**不改任何 `Cargo.toml`**。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! 文件 ──▶ metadata.len() 闸门（[`MAX_AUDIO_FILE_BYTES`]）
//!      ──▶ yeban_render::wav::read_plain_wav（既有读取器）⇒ 实测事实（帧数／采样率／声道／位深）
//!      ──▶ AssetHash::of_bytes（模型的内容寻址键，⛔ 不接受调用方声明）
//!      ──▶ Op::AddClip{ ClipContent::Audio{ asset, gain_db: 0.0 } }（可逆：AddClip ⇄ RemoveClip）
//!      ──▶ **一次** UndoPort::commit_ops_with_origin(now_ms, OpOrigin::Import, …, ops)
//!      ──▶ 字节**交回调用方**（[`AudioImportReport::bytes`]）落进容器资产池
//! ```
//!
//! ## ⚠ 三条诚实登记（⛔ 不假装）
//!
//! 1. **格式面比 MCP 那条窄**：本路径只吃 `read_plain_wav` 能读的 **PCM／float WAV**
//!    （16/24-bit 整数、32-bit 浮点）。MCP 的 `yeban_import_audio` 走 `yeban-decode`
//!    （symphonia：wav／ogg／flac／vorbis／adpcm）并过 `PcmBudget` 五道上限 ⇒
//!    **FLAC／OGG 这类文件在本入口会被明确拒绝**（[`AudioImportError::Decode`]）。
//!    要让两个入口的格式面与预算口径完全对齐，需要一条 `yeban-app → yeban-decode`
//!    依赖边 —— 按本仓纪律（根级/依赖图改动归集成者）**本切片没有加**，留作独立裁决。
//! 2. **不摆放**：本片只在 `clip_pool` 登记片段，⛔ 不写 `Op::AddClipPlacement`。
//!    这正是台账已登记的 UI 计划（`docs/ledger/feature-alignment.md:105` 逐字：
//!    「UI 接一条导入动作（同一个 `Op::AddClip` + 同一份 CAS 池）」）。摆放需要一个
//!    **目的地策略**，以及"帧 → tick（向上取整）"的换算 —— 后者的**唯一实现**自
//!    `#247` 起在模型层 [`yeban_model::time_math::frames_to_ticks_ceil`]（`#248`
//!    删掉了 MCP 的那份副本；app 已依赖 `yeban-model`，`record_landing` 就在用）。
//!    因此摆放留作下一片。
//! 3. **撤销不回收资产池里的字节**：会话 CAS 池的字节**不是** `Op` 的载荷（模型的 `Op`
//!    全集没有资产变体）⇒ `AddClip` 的逆只移除池里的条目，⛔ 不会删掉容器的
//!    `assets/{sha256}`。登记进 CAS 的是**容器字节**（内容寻址，重复导入同一份字节
//!    因此在池里只有一条）。这条边界与 MCP 的 `yeban_import_audio` **同款**，那里也如实
//!    写在响应的 `notes` 里。
//!
//! ## 幂等（按内容，不靠幂等键）
//!
//! 池里已有**同一份字节**（`ClipContent::Audio { asset: 同一 sha256, gain_db: 0.0 }`）
//! ⇒ 复用那条片段、**不发** `Op::AddClip`（`created: false`），因此重复导入同一份文件
//! 不会往 `clip_pool` 里堆第二条同内容的记录。⛔ 但本函数**不**按名字判等 —— 两个不同
//! 文件取了同一个名字就是两条片段（`EntityId` 才是键，`name` 不是）。

use std::path::{Path, PathBuf};

use yeban_model::{AssetHash, ClipContent, ClipPoolEntry, EntityId, Op, OpOrigin, YebanProjectV1};
use yeban_render::wav::read_plain_wav;

use crate::undo::UndoPort;

/// 单个音频文件的**读取上限**（字节）。
///
/// `read_plain_wav` 会把整份 PCM 读进内存 ⇒ 先看 `metadata.len()` 再决定读不读；
/// **超过上限就拒绝**，⛔ 绝不静默截断（截断会让"工程里那份"与"文件里那份"不是同一段
/// 音频，且没人知道）。取值与理由同 `crates/yeban-mcp/src/domain/sfz.rs` 的
/// `MAX_SAMPLE_FILE_BYTES`（64 MiB）—— 那条是"`read_plain_wav` 之前先看文件长度"
/// 的既有口径，本模块照同一条口径办（⛔ 不是新政策）。
pub const MAX_AUDIO_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 导入选项。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioImportOptions {
    /// 片段显示名；`None` = 用文件名的 stem（`kick.wav` ⇒ `kick`）。
    pub name: Option<String>,
}

/// 一次解码的**实测事实**（⛔ 不是估算）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioImportFacts {
    /// 帧数（= 样本数 / 声道数）。
    pub frames: u64,
    /// 文件自身的采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 每样本**有效**位深（16／24／32）。
    pub bits_per_sample: u16,
    /// `true` = IEEE 浮点容器。
    pub is_float: bool,
}

/// 导入计划（提交之前的**全部**产物）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioImportPlan {
    /// 片段显示名。
    pub name: String,
    /// 内容寻址键（源字节的 SHA-256）。
    pub asset: AssetHash,
    /// 源文件的**完整容器字节**（要落进容器资产池的就是它）。
    pub bytes: Vec<u8>,
    /// 解码事实。
    pub facts: AudioImportFacts,
    /// 要（或已经）在 `clip_pool` 里的条目。
    pub clip: ClipPoolEntry,
    /// `true` = 池里没有同内容条目 ⇒ [`Self::ops`] 里有一条 `Op::AddClip`。
    pub clip_is_new: bool,
    /// 待提交的操作（空 = 幂等命中，一位都不改）。
    pub ops: Vec<Op>,
}

/// 导入报告（CLI 打印它；判据从它读回事实）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioImportReport {
    /// 来源文件路径。
    pub path: PathBuf,
    /// 片段显示名。
    pub name: String,
    /// 内容寻址键（源字节的 SHA-256）。
    pub asset: AssetHash,
    /// 要落进容器资产池的字节（调用方负责登记）。
    pub bytes: Vec<u8>,
    /// 解码事实。
    pub facts: AudioImportFacts,
    /// 片段身份（新建或复用的那一条）。
    pub clip_id: EntityId,
    /// `true` = 本次真的登记了新片段；`false` = 池里已有同内容条目（幂等命中）。
    pub created: bool,
    /// 本次提交的身份；`None` = 没有提交（幂等命中）。
    pub commit: Option<EntityId>,
    /// 本次编辑的来源标记（本切片恒为 [`OpOrigin::Import`]）。
    pub origin: OpOrigin,
}

impl AudioImportReport {
    /// 一行可读摘要（CLI 打印它；键值对以空白分隔）。
    ///
    /// ⚠ 与 `imported-midi:` 同一条口径：值**不加引号**（路径／名字里有空白时这一行
    /// 不再可机械切分 —— 需要精确解析的调用方走结构化的 [`AudioImportReport`]，
    /// ⛔ 不解析这一行）。
    #[must_use]
    pub fn summary(&self) -> String {
        let origin = origin_short_name(&self.origin);
        format!(
            "imported-audio: path={} name={} asset={} frames={} rate={} channels={} bits={} \
             float={} asset_bytes={} clip={} created={} commit={} origin={origin}",
            self.path.display(),
            self.name,
            self.asset,
            self.facts.frames,
            self.facts.sample_rate,
            self.facts.channels,
            self.facts.bits_per_sample,
            self.facts.is_float,
            self.bytes.len(),
            self.clip_id.to_canonical_string(),
            self.created,
            self.commit
                .map_or_else(|| "<none>".to_owned(), |id| id.to_canonical_string()),
        )
    }
}

/// `OpOrigin` 的可检索短名（与 `import_midi.rs` 的口径一致；只有本切片用到的取值）。
fn origin_short_name(origin: &OpOrigin) -> &'static str {
    match origin {
        OpOrigin::Import => "import",
        OpOrigin::UserUi => "user-ui",
        OpOrigin::MidiInput => "midi-input",
        OpOrigin::UndoRedo => "undo-redo",
        OpOrigin::AutomationRecord => "automation-record",
        OpOrigin::Migration => "migration",
        OpOrigin::McpProposal { .. } | OpOrigin::McpEdit { .. } => "mcp",
    }
}

/// 导入失败的原因（每一种都如实上报）。
#[derive(Debug)]
pub enum AudioImportError {
    /// 读文件失败（打不开／读不动）。
    Read {
        /// 源文件路径。
        path: PathBuf,
        /// 底层 I/O 错误。
        source: std::io::Error,
    },
    /// 给的路径不是普通文件（例如目录）。
    NotAFile {
        /// 源文件路径。
        path: PathBuf,
    },
    /// 文件超过 [`MAX_AUDIO_FILE_BYTES`] ⇒ **拒绝读取**，⛔ 不截断。
    TooLarge {
        /// 源文件路径。
        path: PathBuf,
        /// 实际字节数。
        bytes: u64,
        /// 上限（字节）。
        max: u64,
    },
    /// 从路径推不出片段名（没有文件名 / stem 为空）⇒ 拒绝，⛔ 不猜一个名字。
    NoFileName {
        /// 源文件路径。
        path: PathBuf,
    },
    /// 解码失败（不是 `read_plain_wav` 能读的 PCM／float WAV，或样本越界）。
    Decode {
        /// 源文件路径。
        path: PathBuf,
        /// 读取器的原样裁决。
        reason: String,
    },
    /// 会话/权威拒绝提交。
    Refused(String),
}

impl core::fmt::Display for AudioImportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "读 `{}` 失败: {source}", path.display())
            }
            Self::NotAFile { path } => write!(formatter, "`{}` 不是普通文件", path.display()),
            Self::TooLarge { path, bytes, max } => write!(
                formatter,
                "`{}` 有 {bytes} 字节, 超过单文件上限 {max} 字节 (拒绝读取, ⛔ 不截断)",
                path.display()
            ),
            Self::NoFileName { path } => write!(
                formatter,
                "`{}` 推不出片段名 (没有文件名或 stem 为空)",
                path.display()
            ),
            Self::Decode { path, reason } => write!(
                formatter,
                "解码 `{}` 失败 (本入口只吃 PCM/float WAV): {reason}",
                path.display()
            ),
            Self::Refused(reason) => write!(formatter, "提交被拒: {reason}"),
        }
    }
}

impl std::error::Error for AudioImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::NotAFile { .. }
            | Self::TooLarge { .. }
            | Self::NoFileName { .. }
            | Self::Decode { .. }
            | Self::Refused(_) => None,
        }
    }
}

/// 规划一次导入（**只读**：读文件 ＋ 解码 ＋ 派生操作；⛔ 不碰会话、⛔ 不写盘）。
///
/// # Errors
///
/// 见 [`AudioImportError`]：读失败 / 不是普通文件 / 超上限 / 推不出名字 / 解码失败。
pub fn plan_audio_import(
    path: &Path,
    project: &YebanProjectV1,
    options: &AudioImportOptions,
) -> Result<AudioImportPlan, AudioImportError> {
    // ① 闸门：先看 `metadata.len()`，**再**决定读不读（`read_plain_wav` 会把整份 PCM
    //    读进内存）。⛔ 不截断。
    let metadata = std::fs::metadata(path).map_err(|source| AudioImportError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(AudioImportError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > MAX_AUDIO_FILE_BYTES {
        return Err(AudioImportError::TooLarge {
            path: path.to_path_buf(),
            bytes: metadata.len(),
            max: MAX_AUDIO_FILE_BYTES,
        });
    }

    let name = match options.name.as_ref() {
        Some(explicit) => explicit.clone(),
        None => path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .filter(|stem| !stem.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| AudioImportError::NoFileName {
                path: path.to_path_buf(),
            })?,
    };

    let bytes = std::fs::read(path).map_err(|source| AudioImportError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    // ② **真解码**：拿事实（并在"这不是本仓能读的音频"时明确拒绝 —— ⛔ 不登记一个
    //    渲染时才发现是坏的资产）。
    let (format, buffer) = read_plain_wav(path).map_err(|error| AudioImportError::Decode {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let channels = format.channels.max(1);
    #[allow(clippy::cast_possible_truncation)]
    let frames = (buffer.len() as u64) / u64::from(channels);
    let facts = AudioImportFacts {
        frames,
        sample_rate: format.sample_rate,
        channels: format.channels,
        bits_per_sample: format.bits_per_sample,
        is_float: format.is_float,
    };

    // ③ 内容寻址：哈希由**字节**算出，⛔ 不接受调用方声明。
    let asset = AssetHash::of_bytes(&bytes);

    // ④ 幂等：池里已有**同内容**（同一 sha256 ＋ 单位增益）的音频条目 ⇒ 复用它。
    let existing = project.clip_pool.values().find(|entry| {
        matches!(
            &entry.content,
            ClipContent::Audio { asset: known, gain_db }
                if *known == asset && *gain_db == 0.0
        )
    });
    let (clip, clip_is_new) = match existing {
        Some(entry) => (entry.clone(), false),
        None => (
            ClipPoolEntry {
                id: EntityId::new(),
                name,
                content: ClipContent::Audio {
                    asset: asset.clone(),
                    gain_db: 0.0,
                },
            },
            true,
        ),
    };
    // 名字以**实际落进池子**的那条为准（幂等命中时是既有条目的名字）。
    let name = clip.name.clone();
    let ops = if clip_is_new {
        vec![Op::AddClip { clip: clip.clone() }]
    } else {
        Vec::new()
    };

    Ok(AudioImportPlan {
        name,
        asset,
        bytes,
        facts,
        clip,
        clip_is_new,
        ops,
    })
}

/// 从**文件**导入（CLI／GUI 路径）：规划 ⇒ **一次**带来源的提交。
///
/// 提交的来源恒为 [`OpOrigin::Import`]（"外部工程/格式导入"，⛔ 不是 `UserUi`）——
/// 这一段编辑是"把一个外部格式读进活跃工程"，来源标签要如实。幂等命中（池里已有同内容
/// 条目）时**不发**提交，[`AudioImportReport::commit`] 为 `None`。
///
/// ⚠ 调用方**必须**把 [`AudioImportReport::bytes`] 登记进容器的资产池
/// （`ProjectArchive.assets` ⇒ `assets/{sha256}`），否则工程文档指向的字节不在容器里。
/// 本函数刻意不碰归档：CAS 池不是 `Op` 的载荷（见模块文档第 3 条）。
///
/// # Errors
///
/// [`AudioImportError`]：规划的每一种失败 ＋ 提交被拒（[`AudioImportError::Refused`]）。
pub fn import_audio_file(
    port: &UndoPort,
    path: &Path,
    project: &YebanProjectV1,
    options: &AudioImportOptions,
    now_ms: u64,
) -> Result<AudioImportReport, AudioImportError> {
    let plan = plan_audio_import(path, project, options)?;
    let commit = if plan.ops.is_empty() {
        None
    } else {
        Some(
            port.commit_ops_with_origin(now_ms, OpOrigin::Import, "import audio", plan.ops)
                .map_err(|refusal| AudioImportError::Refused(refusal.to_string()))?,
        )
    };
    Ok(AudioImportReport {
        path: path.to_path_buf(),
        name: plan.name,
        asset: plan.asset,
        bytes: plan.bytes,
        facts: plan.facts,
        clip_id: plan.clip.id,
        created: plan.clip_is_new,
        commit,
        origin: OpOrigin::Import,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::container::write_project_container;
    use yeban_render::dither::PcmBuffer;
    use yeban_render::rf64::PcmFormat;

    use crate::undo::{UndoPort, UndoSession};

    /// 一次性的临时目录（与 `cli.rs` 的判据同款：⛔ 不用外部 crate）。
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-import-audio-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 现造一份真 WAV（本仓唯一写出器）；返回 `(路径, 字节)`。
    fn write_wav(dir: &Path, frames: usize) -> (PathBuf, Vec<u8>) {
        let path = dir.join("take.wav");
        let samples: Vec<i16> = (0..frames)
            .map(|index| i16::try_from(index % 64).expect("夹具样本") * 100 - 3_000)
            .collect();
        yeban_render::wav::write_plain_wav(
            &path,
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let bytes = std::fs::read(&path).expect("读夹具 WAV");
        (path, bytes)
    }

    /// 判据（**幂等按内容**）：同一份字节导入两次 ⇒ 池里只有一条同内容片段，
    /// 第二次**不发**提交（`created: false` ＋ `commit: None`），工程一位都不改。
    ///
    /// ⛔ 这条**不**证明"两次导入一定得到同一个 `EntityId`"—— 第一次建的片段被复用，
    /// 因此 id 相同；但那是结果不是前提（判据断言的是池子的**条数**与提交的有无）。
    #[test]
    fn importing_the_same_bytes_twice_keeps_one_clip_and_commits_once() {
        let dir = scratch_dir("idempotent");
        let (wav, bytes) = write_wav(&dir, 128);
        let project = crate::bridge::demo_project();
        let session = UndoSession::open(
            "<test:import-audio>",
            "yeban-app",
            project.clone(),
            1_760_000_000_000,
        )
        .expect("开会话");
        let port = UndoPort::new(session);

        let first = import_audio_file(
            &port,
            &wav,
            &project,
            &AudioImportOptions::default(),
            1_760_000_000_001,
        )
        .expect("第一次导入必须成功");
        assert!(first.created, "池里本来没有 ⇒ 必须新建");
        assert!(first.commit.is_some(), "新建 ⇒ 必须有一次提交");
        assert_eq!(first.facts.frames, 128);
        assert_eq!(first.asset, AssetHash::of_bytes(&bytes));

        let after_first = port.project();
        let second = import_audio_file(
            &port,
            &wav,
            &after_first,
            &AudioImportOptions::default(),
            1_760_000_000_002,
        )
        .expect("第二次导入必须成功");
        assert!(!second.created, "同内容已在池里 ⇒ 复用");
        assert_eq!(second.commit, None, "幂等命中 ⇒ 一位都不改，没有提交");
        assert_eq!(second.clip_id, first.clip_id, "复用同一个片段身份");

        // ⚠ 夹具工程**自己**带一条音频片段（`demo_project` 的假哈希，见
        // `yeban-model/src/samples.rs:876`）⇒ 这里数的是**这份字节**的条目，⛔ 不是
        // "工程里有几条音频片段"。
        let same_bytes = |project: &YebanProjectV1| {
            project
                .clip_pool
                .values()
                .filter(|entry| match &entry.content {
                    ClipContent::Audio { asset, .. } => asset.as_str() == first.asset.as_str(),
                    _ => false,
                })
                .count()
        };
        assert_eq!(
            same_bytes(&port.project()),
            1,
            "同内容的音频片段只许有一条（重复导入不得堆第二条）"
        );
    }

    /// 判据（**拒绝不留半成品**）：非音频字节 ⇒ 明确拒绝，池子里**这份字节**一条都没有
    /// （⚠ 夹具工程自带一条**别的**哈希的音频片段，故按哈希数）。
    #[test]
    fn a_non_audio_file_is_refused_and_registers_nothing() {
        let dir = scratch_dir("garbage");
        let junk = dir.join("junk.wav");
        let junk_bytes = b"this is not a RIFF container at all";
        std::fs::write(&junk, junk_bytes).expect("写垃圾");
        let junk_hash = AssetHash::of_bytes(junk_bytes);
        let project = crate::bridge::demo_project();
        let session = UndoSession::open("<test:import-audio-bad>", "yeban-app", project.clone(), 0)
            .expect("开会话");
        let port = UndoPort::new(session);

        let error = import_audio_file(&port, &junk, &project, &AudioImportOptions::default(), 1)
            .expect_err("非音频必须被拒");
        assert!(
            matches!(error, AudioImportError::Decode { .. }),
            "必须是解码层的明确拒绝: {error}"
        );
        assert!(
            !port
                .project()
                .clip_pool
                .values()
                .any(|entry| match &entry.content {
                    ClipContent::Audio { asset, .. } => asset.as_str() == junk_hash.as_str(),
                    _ => false,
                }),
            "被拒 ⇒ 池子里一份这份字节的条目都不许留下"
        );
    }

    /// 判据（**超上限先拒绝，⛔ 不读、⛔ 不截断**）：闸门看的是 `metadata.len()`，
    /// 因此一个"声称很大"的文件在**读之前**就被拒 —— 这里用真文件证明闸门真的在
    /// 读取之前生效（把上限临时改小不现实，故直接断言 `TooLarge` 的读数）。
    #[test]
    fn a_file_over_the_gate_is_refused_with_its_real_byte_count() {
        let dir = scratch_dir("too-large");
        let big = dir.join("big.wav");
        // 用 `set_len` 造一个**稀疏**的大文件：不占真实磁盘，但 `metadata.len()` 是真的。
        let file = std::fs::File::create(&big).expect("建文件");
        file.set_len(MAX_AUDIO_FILE_BYTES + 1).expect("设长度");
        drop(file);
        let project = crate::bridge::demo_project();

        let error = plan_audio_import(&big, &project, &AudioImportOptions::default())
            .expect_err("超上限必须被拒");
        match error {
            AudioImportError::TooLarge { bytes, max, .. } => {
                assert_eq!(bytes, MAX_AUDIO_FILE_BYTES + 1);
                assert_eq!(max, MAX_AUDIO_FILE_BYTES);
            }
            other => panic!("必须是 TooLarge: {other}"),
        }
    }

    /// 判据（**内容寻址键来自字节**）：报告里的 `asset` 等于源字节的 SHA-256，
    /// 且与容器写出用的键是同一个 —— 这条钉住"哈希不是调用方声明、也不是文件名派生的"。
    #[test]
    fn the_asset_hash_is_the_sha256_of_the_container_bytes() {
        let dir = scratch_dir("hash");
        let (wav, bytes) = write_wav(&dir, 7);
        let project = crate::bridge::demo_project();
        let plan = plan_audio_import(&wav, &project, &AudioImportOptions::default())
            .expect("规划必须成功");
        assert_eq!(plan.asset, AssetHash::of_bytes(&bytes));
        assert_eq!(plan.bytes, bytes, "落进资产池的必须是**原样的容器字节**");
        assert_eq!(plan.facts.sample_rate, 48_000);
        assert_eq!(plan.facts.channels, 1);
        assert_eq!(plan.facts.bits_per_sample, 16);
        assert_eq!(plan.facts.frames, 7);
        assert_eq!(plan.name, "take", "缺省名 = 文件 stem");
        assert!(plan.clip_is_new);
        // 容器写出真的接受它（⛔ 不靠"能编译"当证据）。
        let archive = yeban_model::container::ProjectArchive {
            project: project.clone(),
            history_dag: Vec::new(),
            assets: vec![(plan.asset.clone(), plan.bytes.clone())],
        };
        let container = write_project_container(
            &archive.project,
            &archive.history_dag,
            &std::collections::BTreeMap::from([(plan.asset.clone(), plan.bytes.clone())]),
        )
        .expect("容器必须接受这份字节");
        assert!(!container.is_empty());
    }
}
