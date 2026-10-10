//! `yeban_import_audio` 的实现 —— 把一份音频**登记成 `clip_pool` 条目**
//! （`ADR-0001` **D46** 的第 3 类能力）。
//!
//! ## 两条来源、一份落点
//!
//! | 来源 | 校验 | 字节从哪来 |
//! | :--- | :--- | :--- |
//! | `assetHash`（已在容器资产池 `assets/{sha256}`） | 池里**必须真的**有这个哈希的字节（否则 `ENTITY_NOT_FOUND`） | 池里已有的字节（**不**重复登记） |
//! | `path`（磁盘文件） | 先按 [`PcmBudget`] 过闸门再解码 | 一次 `std::fs::read` 的容器字节 |
//!
//! 两条来源**恰好给一个**（形状判定在 [`super::extension_pure::select_import_source`]）；
//! 两个都给或都不给都是 `INVALID_PARAMETER_RANGE`，绝不"猜一个"。
//!
//! ## 走既有 `yeban-decode`，并**尊重 `PcmBudget`**
//!
//! 两条来源都会真的解码一遍（[`DecodeOptions::default`]，与 `yeban_render_master` 的
//! 音频片段路径**同一个**预算与**同一个**资产池读法 [`render::AssetStore`]）：
//!
//! - 解码失败 ⇒ 明确错误，**不**登记一个"渲染时才发现是坏的"资产；
//! - 超预算 ⇒ `INVALID_PARAMETER_RANGE` + `data.budget = true` + 生效的五道上限
//!   （预算的闸门都发生在**分配之前**，见 `yeban-decode` 的模块文档）；
//! - 解出的 PCM 只用于**读事实**，随即释放 —— 登记进 CAS 池的是**容器字节**
//!   （渲染路径自己会解码 `ClipContent::Audio` 的资产字节，因此池里必须是原始容器）。
//!
//! ## 落点是 `Op::AddClip`（因此可逆）
//!
//! 登记本身是一条领域操作：`clip_pool` 里多一条 `ClipContent::Audio { asset, gain_db }`。
//! 因此撤销是模型自己的 [`Op::invert`]（`AddClip` ⇄ `RemoveClip`），本层**不写**逆操作。
//!
//! ## 摆放：`trackId` 一给，同一次调用就把片段落到轨道上（`Op::AddClipPlacement`）
//!
//! 只登记片段是**不可渲染**的：渲染只遍历 `track.clips`（`render.rs` 的
//! `audio_clip_source`），`clip_pool` 里没有摆放引用的条目在母带里**一帧都不出现**。
//! 模型的 `Op` 全集里**已经有** [`Op::AddClipPlacement`]（`crates/yeban-model/src/ops.rs`），
//! 缺的只是工具面：本工具此前**只有** `AddClip` 这一半。
//!
//! 因此本工具新增五个**可选**实参，形状与既有参数一致（缺省 = 一位都不改）：
//!
//! | 实参 | 缺省 | 语义 |
//! | :--- | :--- | :--- |
//! | `trackId` | 不给 | 目标音轨；**给了才摆放**（不给 = 只登记，与从前逐字节相同） |
//! | `startTick` | `0` | 摆放起点（tick） |
//! | `durationTicks` | 由素材全长换算 | 摆放时值（tick，`>= 1`）；这是**剪辑边界**（硬切） |
//! | `placementId` | 由 `(片段, 音轨, 起点)` 确定性派生 | 摆放的显式身份 |
//! | `muted` | `false` | 摆放是否静音 |
//!
//! 三条刻意的口径：
//!
//! 1. **不给 `trackId` 就不许给其余四个**（`INVALID_PARAMETER_RANGE` +
//!    `data.reason = "placementWithoutTrack"`）：没有轨道就无处摆放，静默忽略会让
//!    调用方以为片段已经落轨；
//! 2. **时值缺省是"素材全长"**：`ceil(frames * PPQ * bpm / (sampleRate * 60))`（见
//!    [`super::render_math::frames_to_ticks_ceil`]）。向上取整保证尾部的不足一 tick
//!    不被切掉；工程速度不可用时**明确拒绝**，而不是写一个会被模型层拒掉的 `0`；
//! 3. **确认（幂等）按"片段 + 摆放"两段各自判定**：片段逐字段相同就不重复登记，
//!    摆放逐字段相同就不重复摆放。两次调用因此**一位都不改**（`unchanged: true`）。
//!    同身份但内容不同（片段或摆放）一律 `CONFLICT`。
//!
//! 两条 `Op` 在**同一次提交**里按 `AddClip` → `AddClipPlacement` 的顺序施加
//! （`undo_session::commit` 用 `Op::Batch` 包住它们，模型层的 `Batch` 是顺序施加 +
//! 整体原子），因此"片段还没进池子就先摆放"这个中间态不可能被观察到。
//!
//! ### 操作来源：`OpOrigin::McpEdit`（不再借 `Import`）
//!
//! 本工具**直接**在活跃工程上落一条片段、**不**创建提案 ⇒ 作者标签是
//! [`OpOrigin::McpEdit`]（"MCP 代理的直接编辑"）。此前借的 `Import`
//! （"外部工程/格式导入"）描述的不是"代理直接改活跃工程"这件事。
//! `agent_name` 取 [`super::AGENT_NAME`]（与 `UndoState.author` 同源）。
//! 契约**已承认**该分支（`origin.oneOf[2]` = `McpEdit`，第 82 轮起）。
//!
//! ### 撤销**不**回收 CAS 池里的字节（如实登记的边界）
//!
//! 会话 CAS 池（`assets/{sha256}`）的字节**不是** `Op` 的载荷 —— 模型的 `Op` 全集里
//! 没有任何资产变体（实测 29 个变体，`SetAsset` / `DeclareAsset` 都不存在）。于是：
//!
//! - 内容寻址 ⇒ 重复导入**同一份字节**幂等（池里只有一条），"孤儿字节"因此是可收敛的；
//! - 但 `AddClip` 的逆**不会**删掉池里的字节 ⇒ 撤销后保存会把一份未被引用的资产写进容器。
//!   这是模型的缺口（needs-2 / needs-3），本工具**如实**在响应的 `notes` 里写出来，
//!   而不是假装撤销把一切都收回去了。
//!
//! ## 幂等（按**内容与意图**，不靠幂等键）
//!
//! 片段身份缺省由 `(来源, 名字, 增益)` **确定性派生**（[`super::extension_pure::clip_label`]），
//! 因此重复导入同一意图会命中**同一条** `clip_pool` 记录：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 池里没有该身份 | 真登记（一条 `Op::AddClip`） |
//! | 池里有**逐字段相同**的条目 | `created: false` + `unchanged: true`，**一位都不改** |
//! | 池里有同身份但**内容不同**的条目 | `CONFLICT`（绝不静默覆盖别人的片段） |

use serde_json::{Map, Value};

use yeban_decode::{DecodeError, DecodeOptions, DecodedAsset};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, Op, OpOrigin, PPQ,
    YebanProjectV1,
};

use super::error::{self, Fault};
use super::extension_pure::{self, ImportSource};
use super::ids::deterministic_id;
use super::render::AssetStore;
use super::render_math;
use crate::tools::{ErrorCode, ToolResponse};

/// 一次音频导入的**完整只读规划**（`dryRun` 与真做共用）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioImport {
    /// 将要（或已经）登记进 `clip_pool` 的条目。
    pub clip: ClipPoolEntry,
    /// 将要提交的领域操作（**顺序即施加顺序**：`AddClip` 在 `AddClipPlacement` 之前）。
    ///
    /// 空 = 片段与摆放都已逐字段存在（幂等命中，一位都不改）。
    pub ops: Vec<Op>,
    /// 将要（或已经）落到轨道上的摆放；`None` = 本次没有要求摆放。
    pub placement: Option<ClipPlacement>,
    /// 摆放的目标音轨（`placement.is_some()` 时必定 `Some`）。
    pub placement_track: Option<EntityId>,
    /// 摆放时值是否由**素材全长**换算而来（`false` = 调用方显式给出）。
    pub placement_duration_derived: bool,
    /// 来源类别。
    pub source: ImportSource,
    /// 来源的稳定引用（`asset:<sha256>` 或 `disk:<path>`）。
    pub source_ref: String,
    /// 资产哈希（内容寻址的键；由字节算出，**不接受**调用方声明）。
    pub asset: AssetHash,
    /// 需要登记进会话 CAS 池的**容器字节**（`AssetPool` 来源时为 `None`）。
    pub bytes: Option<Vec<u8>>,
    /// 该资产是否已经在工程 `assets` 索引里声明。
    pub declared_in_index: bool,
    /// 解码事实（实测，不是估算）。
    pub facts: DecodeFactsValue,
    /// 写入前的工程内容摘要。
    pub digest_before: String,
    /// 写入后的工程内容摘要（只读规划在克隆体上算出来的预测；幂等命中时为 `None`）。
    pub digest_after: Option<String>,
}

/// 一份资产的**实测**解码事实（响应 `data.decoded`）。
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeFactsValue {
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 解出的帧数。
    pub frames: u64,
    /// 时长（秒）。
    pub duration_seconds: f64,
    /// 实际解出的样本格式短名（`u8` / `s16` / `f32` …）。
    pub pcm_format: &'static str,
    /// 模型位深名（`Int16` / `Int24` / `Float32`），不可表达时为 `None`。
    pub model_bit_depth: Option<String>,
    /// 交织 PCM 的 SHA-256（同一输入 ⇒ 同一摘要）。
    pub pcm_hash: String,
    /// 容器声明的总帧数（没声明时 `None`）。
    pub declared_frames: Option<u64>,
    /// 声明时长与解出时长是否已对账通过。
    pub duration_reconciled: bool,
}

impl DecodeFactsValue {
    /// 由解码产物读事实（**唯一**的读取处；调用方随即丢弃 `DecodedAsset`）。
    #[must_use]
    pub fn of(asset: &DecodedAsset) -> Self {
        Self {
            sample_rate: asset.sample_rate(),
            channels: asset.channels(),
            frames: asset.frame_count(),
            duration_seconds: asset.duration_seconds(),
            pcm_format: super::render::pcm_format_name(asset.pcm_format()),
            model_bit_depth: asset.model_bit_depth().map(|depth| {
                serde_json::to_value(depth)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".to_owned())
            }),
            pcm_hash: asset.pcm_hash().as_str().to_owned(),
            declared_frames: asset.declared_frames(),
            duration_reconciled: asset.duration_is_reconciled(),
        }
    }

    /// JSON 形态。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "sampleRate": self.sample_rate,
            "channels": self.channels,
            "frames": self.frames,
            "durationSeconds": self.duration_seconds,
            "pcmFormat": self.pcm_format,
            "modelBitDepth": self.model_bit_depth.clone().map_or(Value::Null, Value::from),
            "pcmHash": self.pcm_hash.clone(),
            "declaredFrames": self.declared_frames.map_or(Value::Null, Value::from),
            "durationReconciled": self.duration_reconciled,
        })
    }
}

impl AudioImport {
    /// 片段引用的 `(资产哈希文本, 增益)`；非音频内容在结构上不可能出现（明确报错，不 panic）。
    fn asset_ref(&self) -> Result<(String, f32), Fault> {
        match &self.clip.content {
            ClipContent::Audio { asset, gain_db } => Ok((asset.as_str().to_owned(), *gain_db)),
            ClipContent::Midi { .. } => Err(Fault::domain(
                ErrorCode::Conflict,
                "音频导入计划里出现了 MIDI 片段 (内部不变量被破坏)",
            )),
        }
    }

    /// 本次是否会**新增**片段池条目（`Op::AddClip` 在场）。
    ///
    /// 与 `unchanged` 分开：片段已存在、只补一次摆放时 `created` 是 `false` ——
    /// 那一次调用没有新建任何片段，把它报成 `created` 会让调用方以为池里多了一条。
    #[must_use]
    pub fn clip_created(&self) -> bool {
        self.ops.iter().any(|op| matches!(op, Op::AddClip { .. }))
    }

    /// 本次是否会**新增**摆放（`Op::AddClipPlacement` 在场）。
    #[must_use]
    pub fn placement_created(&self) -> bool {
        self.ops
            .iter()
            .any(|op| matches!(op, Op::AddClipPlacement { .. }))
    }

    /// 本次是否一位都不改（片段与摆放都已逐字段存在）。
    #[must_use]
    pub fn unchanged(&self) -> bool {
        self.ops.is_empty()
    }

    /// 响应的 `data`（`dryRun` 预览与真做**共用这一个函数**）。
    ///
    /// # Errors
    ///
    /// 摘要序列化失败 → `IO_ERROR`。
    pub fn data(&self) -> Result<Value, Fault> {
        let (asset, gain_db) = self.asset_ref()?;
        Ok(serde_json::json!({
            "imported": true,
            "created": self.clip_created(),
            "unchanged": self.unchanged(),
            "clip": {
                "clipId": self.clip.id.to_canonical_string(),
                "name": self.clip.name.clone(),
                "asset": asset,
                "gainDb": gain_db,
                "contentKind": "Audio",
            },
            "placement": self.placement_value(),
            "source": {
                "kind": match self.source {
                    ImportSource::AssetPool => "assetPool",
                    ImportSource::DiskPath => "diskPath",
                },
                "ref": self.source_ref.clone(),
                "registeredBytes": self.bytes.as_ref().map_or(0, Vec::len),
            },
            "decoded": self.facts.to_value(),
            "budget": budget_value(),
            "declaredInIndex": self.declared_in_index,
            "projectDigestBefore": self.digest_before.clone(),
            "projectDigestAfter": self.digest_after.clone().map_or(Value::Null, Value::from),
            "notes": notes(
                self.bytes.is_some(),
                self.placement.as_ref().map(|_| self.placement_created()),
            ),
        }))
    }

    /// `data.placement`：没要求摆放时是 `null`（**不是**一个"全零摆放"）。
    fn placement_value(&self) -> Value {
        let Some(placement) = &self.placement else {
            return Value::Null;
        };
        serde_json::json!({
            "placementId": placement.id.to_canonical_string(),
            "trackId": self
                .placement_track
                .map_or(Value::Null, |track| Value::from(track.to_canonical_string())),
            "startTick": placement.start_tick,
            "durationTicks": placement.duration_ticks,
            "muted": placement.muted,
            "placed": self.placement_created(),
            "durationRule": if self.placement_duration_derived {
                "素材全长换算: ceil(frames * PPQ * bpm / (sampleRate * 60))"
            } else {
                "调用方显式给出的 durationTicks"
            },
        })
    }
}

/// 生效的 `PcmBudget`（**唯一**来源：`yeban_decode::DecodeOptions::default()`）。
///
/// 刻意**不**在这里抄一份数字：预算由 `yeban-decode` 的模块文档推导，抄一份必然漂移。
#[must_use]
pub fn budget_value() -> Value {
    let budget = DecodeOptions::default().budget;
    serde_json::json!({
        "gate": "yeban_decode::DecodeOptions::default()",
        "maxInputBytes": budget.max_input_bytes,
        "maxPcmBytes": budget.max_pcm_bytes,
        "maxChannels": budget.max_channels,
        "maxSampleRate": budget.max_sample_rate,
        "maxDurationSecs": budget.max_duration_secs,
        "interleavedSamplesLimit": budget.interleaved_samples_limit(),
    })
}

/// 如实登记的边界（不藏在错误码后面）。
///
/// `placement_created` 是**三态**：`None` = 本次没要求摆放，`Some(true)` = 本次真的提交了
/// `Op::AddClipPlacement`，`Some(false)` = 摆放已逐字段存在（未重复提交）。
/// 用三态而不是"要求了就写提交"：后者会在幂等重放时**谎报**一次提交。
fn notes(registered_bytes: bool, placement_created: Option<bool>) -> Vec<&'static str> {
    let mut notes = vec![
        "CAS 池的字节不是 Op 的载荷 (模型 Op 全集没有资产变体) ⇒ 撤销 clip_pool 条目不会回收池里的字节 (needs-2)",
        "工程 assets 索引 (许可/原路径/字节数) 同样没有 Op ⇒ 本工具只登记 clip_pool 条目, 不写索引 (needs-3): 索引是许可留痕 (MUST-GATE-014) 而非字节来源, 渲染读的是会话 CAS 池",
    ];
    if registered_bytes {
        notes.push("本次真的向会话 CAS 池登记了容器字节 (内容寻址: 同一份字节不会重复登记)");
    } else {
        notes.push("字节已在会话 CAS 池里 (按哈希引用, 未重复登记)");
    }
    match placement_created {
        Some(true) => notes.push(
            "本次同时提交了 Op::AddClipPlacement ⇒ 片段真的落在轨道上 (渲染只遍历 track.clips; \
             只有 clip_pool 条目的片段在母带里一帧都不会出现)",
        ),
        Some(false) => notes.push("摆放已逐字段存在 ⇒ 未重复提交 Op::AddClipPlacement (幂等命中)"),
        None => {
            notes.push("本次没有要求摆放 (未给 trackId) ⇒ 片段只在 clip_pool 里, 渲染不会遍历到它")
        }
    }
    notes
}

/// 规划一次音频导入（只读；`dryRun` 走的就是它）。
///
/// # Errors
///
/// - 来源形状非法（两个都给/都不给）→ `INVALID_PARAMETER_RANGE`；
/// - 池里没有该资产 → `ENTITY_NOT_FOUND`；
/// - 磁盘文件不存在 → `FILE_NOT_FOUND`；读失败 → `IO_ERROR` / `DISK_FULL`；
/// - 解码失败 → `RENDER_FAILED`（+ `data.decodeError` 分类）；
/// - 超出 `PcmBudget` → `INVALID_PARAMETER_RANGE`（+ `data.budget = true`）；
/// - 同身份但内容不同的片段已存在 → `CONFLICT`；
/// - 摆放形状非法（给了 `startTick`/`durationTicks`/`placementId`/`muted` 却没给 `trackId`、
///   `durationTicks == 0`、工程速度无法换算时值）→ `INVALID_PARAMETER_RANGE`；
/// - 摆放的目标音轨不存在 → `TRACK_NOT_FOUND`；同身份但内容不同的摆放已存在 → `CONFLICT`；
/// - 增益/身份形状非法 → `INVALID_PARAMETER_RANGE`。
pub fn plan(
    project: &YebanProjectV1,
    assets: &dyn AssetStore,
    arguments: &Map<String, Value>,
) -> Result<AudioImport, Fault> {
    let name = arguments
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "`name` 必须是字符串"))?;
    let asset_hash_arg = arguments.get("assetHash").and_then(Value::as_str);
    let path_arg = arguments.get("path").and_then(Value::as_str);
    let source =
        extension_pure::select_import_source(asset_hash_arg, path_arg).map_err(|error| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                error.message(),
                serde_json::json!({ "reason": error.as_str() }),
            )
        })?;
    let gain_db = parse_gain(arguments)?;

    // ---- 取字节 + 解码（两条来源都要真的解码一遍） ----
    let (bytes, facts, asset, source_ref, declared_in_index) = match source {
        ImportSource::AssetPool => {
            let text = asset_hash_arg.unwrap_or_default();
            let hash = AssetHash::parse(text).map_err(|error| {
                Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("`assetHash` 不是合法 SHA-256: {error}"),
                    serde_json::json!({ "field": "assetHash" }),
                )
            })?;
            let Some(existing) = assets.asset(&hash) else {
                return Err(Fault::domain_with_data(
                    ErrorCode::EntityNotFound,
                    format!(
                        "会话资产池里没有资产 {} —— 先用 `path` 导入磁盘文件, \
                         或确认容器里真的有 assets/{}",
                        hash.as_str(),
                        hash.as_str()
                    ),
                    serde_json::json!({
                        "reason": "assetNotInPool",
                        "asset": hash.as_str(),
                        "hint": "`assetHash` 只接受**已经在**会话 CAS 池里的资产",
                    }),
                ));
            };
            let facts = decode_and_measure(existing, &format!("资产 {}", hash.as_str()))?;
            let declared = project.assets.contains_key(&hash);
            // ⚠ `AssetHash` **不是 `Copy`**：来源标签必须在把 `hash` 移进元组**之前**算好
            // （否则 `hash.as_str()` 就是"borrow of moved value"，CI 的 E0382 实测抓过）。
            let source_ref = format!("asset:{}", hash.as_str());
            (None, facts, hash, source_ref, declared)
        }
        ImportSource::DiskPath => {
            let text = path_arg.unwrap_or_default();
            let bytes = std::fs::read(text)
                .map_err(|error| error::from_io(&format!("读取 {text}"), &error))?;
            let facts = decode_and_measure(&bytes, &format!("文件 {text}"))?;
            // 内容寻址: 哈希由**字节**算出, 不接受调用方声明 (与 `Domain::put_asset` 同一口径)。
            let hash = AssetHash::of_bytes(&bytes);
            let declared = project.assets.contains_key(&hash);
            (Some(bytes), facts, hash, format!("disk:{text}"), declared)
        }
    };

    // ---- 片段身份（显式或确定性派生） ----
    let clip_id = match arguments.get("clipId") {
        Some(raw) => {
            let text = raw.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`clipId` 必须是 26 字符 ULID 字符串",
                )
            })?;
            parse_entity("clipId", text)?
        }
        None => deterministic_id(&extension_pure::clip_label(&source_ref, name, gain_db)),
    };
    let clip = ClipPoolEntry {
        id: clip_id,
        name: name.to_owned(),
        content: ClipContent::Audio {
            asset: asset.clone(),
            gain_db,
        },
    };

    // ---- 摆放（可选；`trackId` 一给就摆放） ----
    let placement = plan_placement(project, arguments, clip_id, &facts)?;

    // ---- 幂等 / 冲突（按内容与意图，不靠幂等键；片段与摆放各自判定） ----
    let mut ops: Vec<Op> = Vec::new();
    match project.clip_pool.get(&clip_id) {
        Some(existing) if *existing == clip => {
            // 逐字段相同 ⇒ 不重复登记。
        }
        Some(existing) => {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!("片段池里已有身份 {clip_id}, 但内容不同"),
                serde_json::json!({
                    "clipId": clip_id.to_canonical_string(),
                    "existing": serde_json::to_value(existing).unwrap_or(Value::Null),
                    "requested": serde_json::to_value(&clip).unwrap_or(Value::Null),
                    "hint": "换一个 `clipId`, 或换一个 `name`/`gainDb` (它们参与身份派生)",
                }),
            ));
        }
        None => ops.push(Op::AddClip { clip: clip.clone() }),
    }
    if let Some(request) = &placement {
        let track = project
            .track(&request.track)
            .map_err(|error| error::from_model("摆放的目标音轨", &error))?;
        match track.clips.get(&request.placement.id) {
            Some(existing) if *existing == request.placement => {
                // 逐字段相同 ⇒ 不重复摆放。
            }
            Some(existing) => {
                return Err(Fault::domain_with_data(
                    ErrorCode::Conflict,
                    format!(
                        "音轨 {} 上已有身份 {}, 但摆放内容不同",
                        request.track, request.placement.id
                    ),
                    serde_json::json!({
                        "trackId": request.track.to_canonical_string(),
                        "placementId": request.placement.id.to_canonical_string(),
                        "existing": serde_json::to_value(existing).unwrap_or(Value::Null),
                        "requested": serde_json::to_value(request.placement).unwrap_or(Value::Null),
                        "hint": "换一个 `placementId`/`startTick`, 或用同一条摆放的逐字段相同载荷重放",
                    }),
                ));
            }
            None => ops.push(Op::AddClipPlacement {
                track_id: request.track,
                placement: request.placement,
            }),
        }
    }

    let mut digest_after = None;
    if !ops.is_empty() {
        // 与 `undo_session::commit` **同一口径**：`Batch` 是按顺序施加的，
        // 因此这里也按顺序施加，好让预览的预测逐字节等于真做后的实测摘要。
        let mut simulated = project.clone();
        for op in &ops {
            op.apply(&mut simulated)
                .map_err(|error| error::from_model("音频片段登记模拟", &error))?;
        }
        simulated
            .validate()
            .map_err(|error| error::from_model("音频片段登记校验", &error))?;
        digest_after = Some(digest_of(&simulated)?);
    }

    Ok(AudioImport {
        clip,
        ops,
        placement_track: placement.as_ref().map(|request| request.track),
        placement: placement.as_ref().map(|request| request.placement),
        placement_duration_derived: placement
            .as_ref()
            .is_some_and(|request| request.duration_derived),
        source,
        source_ref,
        asset,
        bytes,
        declared_in_index,
        facts,
        digest_before: digest_of(project)?,
        digest_after,
    })
}

/// 一次**已校验**的摆放请求（`trackId` 给定时才存在）。
#[derive(Clone, Debug, PartialEq)]
struct PlacementRequest {
    /// 目标音轨。
    track: EntityId,
    /// 摆放载荷。
    placement: ClipPlacement,
    /// 时值是否由素材全长换算而来。
    duration_derived: bool,
}

/// 读摆放实参并构造摆放载荷（`trackId` 不给 ⇒ `None`；给了 ⇒ 四个兄弟实参才有意义）。
///
/// 形状判定是**严格**的：给了 `startTick` / `durationTicks` / `placementId` / `muted`
/// 却不给 `trackId` 一律 `INVALID_PARAMETER_RANGE`（`data.reason = "placementWithoutTrack"`）——
/// 静默忽略会让调用方以为片段已经落轨。
fn plan_placement(
    project: &YebanProjectV1,
    arguments: &Map<String, Value>,
    clip_id: EntityId,
    facts: &DecodeFactsValue,
) -> Result<Option<PlacementRequest>, Fault> {
    let start_raw = arguments.get("startTick");
    let duration_raw = arguments.get("durationTicks");
    let placement_id_raw = arguments.get("placementId");
    let muted_raw = arguments.get("muted");
    let Some(track_raw) = arguments.get("trackId") else {
        if let Some(name) = ["startTick", "durationTicks", "placementId", "muted"]
            .into_iter()
            .find(|name| arguments.contains_key(*name))
        {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "给了 `{name}` 却没有给 `trackId` —— 没有目标音轨就无处摆放; \
                     要么给出 `trackId`, 要么去掉这四个摆放实参"
                ),
                serde_json::json!({ "reason": "placementWithoutTrack", "field": name }),
            ));
        }
        return Ok(None);
    };
    let track_text = track_raw.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`trackId` 必须是 26 字符 ULID 字符串",
        )
    })?;
    let track = parse_entity("trackId", track_text)?;
    // 目标音轨必须**真的存在**（`TrackNotFound`），且它的 `clips` 是摆放的落点。
    project
        .track(&track)
        .map_err(|error| error::from_model("摆放的目标音轨", &error))?;

    let start_tick = parse_optional_u64("startTick", start_raw)?.unwrap_or(0);
    let (duration_ticks, duration_derived) =
        match parse_optional_u64("durationTicks", duration_raw)? {
            Some(0) => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    "`durationTicks` 必须 >= 1 (模型层拒绝零时值的摆放)",
                    serde_json::json!({ "field": "durationTicks", "value": 0 }),
                ));
            }
            Some(value) => (value, false),
            None => {
                let derived = render_math::frames_to_ticks_ceil(
                    facts.frames,
                    PPQ,
                    project.bpm,
                    facts.sample_rate,
                )
                .ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        "工程 BPM 或素材采样率无法把帧数换算成 tick ⇒ 必须显式给出 `durationTicks`",
                        serde_json::json!({
                            "reason": "tempoUnusable",
                            "field": "durationTicks",
                            "bpm": project.bpm,
                            "sampleRate": facts.sample_rate,
                            "frames": facts.frames,
                        }),
                    )
                })?;
                (derived, true)
            }
        };
    let placement_id = match placement_id_raw {
        Some(raw) => {
            let text = raw.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`placementId` 必须是 26 字符 ULID 字符串",
                )
            })?;
            parse_entity("placementId", text)?
        }
        None => deterministic_id(&extension_pure::placement_label(
            &clip_id.to_canonical_string(),
            &track.to_canonical_string(),
            start_tick,
        )),
    };
    let placement = ClipPlacement {
        id: placement_id,
        clip_id,
        start_tick,
        duration_ticks,
        // 循环配置是模型的**必需**子结构 [ADR-0001 D43]：缺省 = 关闭（不重复）。
        // "循环重复"本来就不在渲染的已支持面里（`unsupported: clipLoopRepetition`），
        // 因此这里刻意不暴露 `loopEnabled`：那会给出一个渲染不了的旋钮。
        loop_config: LoopConfig::default(),
        muted: parse_optional_bool("muted", muted_raw)?.unwrap_or(false),
    };
    placement
        .validate()
        .map_err(|error| error::from_model("摆放载荷", &error))?;
    Ok(Some(PlacementRequest {
        track,
        placement,
        duration_derived,
    }))
}

/// 读一个可选的非负整数实参。
fn parse_optional_u64(field: &str, raw: Option<&Value>) -> Result<Option<u64>, Fault> {
    let Some(value) = raw else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是 >= 0 的整数"),
            serde_json::json!({ "field": field, "value": value.clone() }),
        )
    })
}

/// 读一个可选的布尔实参。
fn parse_optional_bool(field: &str, raw: Option<&Value>) -> Result<Option<bool>, Fault> {
    let Some(value) = raw else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是布尔值"),
            serde_json::json!({ "field": field, "value": value.clone() }),
        )
    })
}

/// 解码容器字节，并读回事实（**不**保留 PCM）。
fn decode_and_measure(bytes: &[u8], context: &str) -> Result<DecodeFactsValue, Fault> {
    let decoded = yeban_decode::decode_bytes(bytes, &DecodeOptions::default())
        .map_err(|error| decode_fault(context, &error))?;
    Ok(DecodeFactsValue::of(&decoded))
}

/// [`DecodeError`] → 契约错误码（**复用** `render.rs` 的分类名表，不造第二份词汇表）。
///
/// | 变体 | 码 | 理由 |
/// | :--- | :--- | :--- |
/// | `Io(_)` | `FILE_NOT_FOUND` / `DISK_FULL` / `IO_ERROR`（[`error::code_for_io`]） | 真的是 I/O 失败 |
/// | `Budget(_)` | `INVALID_PARAMETER_RANGE` + `data.budget = true` | `PcmBudget` 是**请求侧**的上限闸门（"这份素材超出可导入的尺寸/时长"），不是解码器故障 |
/// | 其余 | `RENDER_FAILED` + `data.decodeError` | 与 `yeban_render_master` 的音频片段路径**同一个**口径（畸形流/不支持的容器/空流…） |
fn decode_fault(context: &str, error: &DecodeError) -> Fault {
    match error {
        DecodeError::Io(io) => error::from_io(context, io),
        DecodeError::Budget(violation) => Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("{context} 超出解码预算 (PcmBudget): {violation}"),
            serde_json::json!({
                "budget": true,
                "reason": "pcmBudgetExceeded",
                "budgetDetail": format!("{violation:?}"),
                "budgetGate": budget_value(),
            }),
        ),
        other => Fault::domain_with_data(
            ErrorCode::RenderFailed,
            format!("{context} 解码失败: {other}"),
            serde_json::json!({
                "reason": "assetDecodeFailed",
                "decodeError": super::render::decode_error_class(other),
                "detail": other.to_string(),
                "specId": "MODEL-AST-007",
            }),
        ),
    }
}

/// 读 `gainDb` 实参（缺省 0.0；必须有限）。
fn parse_gain(arguments: &Map<String, Value>) -> Result<f32, Fault> {
    let Some(raw) = arguments.get("gainDb") else {
        return Ok(0.0);
    };
    let value = raw
        .as_f64()
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "`gainDb` 必须是数字"))?;
    #[allow(clippy::cast_possible_truncation)]
    let value = value as f32;
    if !value.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`gainDb` 必须有限, 实际 {value}"),
            // 与 `automation::parse_point` 同一口径：`reason` 让"是**哪一道**闸门拒的"
            // 机器可读（`InvalidParameterRange` 还被别的闸门共用）。
            serde_json::json!({
                "field": "gainDb",
                "value": value,
                "reason": "nonFiniteValue",
            }),
        ));
    }
    Ok(value)
}

/// 读一个 `EntityId` 实参。
fn parse_entity(field: &str, text: &str) -> Result<EntityId, Fault> {
    use std::str::FromStr as _;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

/// 工程内容摘要（与 `Domain::project_digest` 同口径）。
fn digest_of(project: &YebanProjectV1) -> Result<String, Fault> {
    let json = super::store::serialize_project(project)?;
    Ok(super::store::digest_of(json.as_bytes()))
}

/// **施加**一次音频导入：先登记 CAS 字节（内容寻址），再提交领域操作
/// （`AddClip` 与可选的 `AddClipPlacement`，**同一次提交**）。
///
/// 顺序是刻意的：**先**把字节放进池子，**再**提交引用它的片段。反过来会让"片段已经存在、
/// 字节还没到"的窗口存在（渲染那一刻会得到一个缺字节的资产）。
///
/// # Errors
///
/// - 没有活跃工程 → `NO_ACTIVE_PROJECT`；
/// - 池登记失败 → 既有领域失败；
/// - 提交被撤销会话拒绝 → 既有映射；
/// - 真实摘要与预览预测不一致 → `CONFLICT`。
pub fn apply(domain: &mut super::Domain, import: &AudioImport) -> Result<ToolResponse, Fault> {
    // 1) 登记字节（只改内存; 持久化发生在下一次 `yeban_save_project`）。
    if let Some(bytes) = &import.bytes {
        let hash = domain.put_asset(bytes.clone())?;
        if hash != import.asset {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                "登记进 CAS 池的字节与计划里的资产哈希不符",
                serde_json::json!({
                    "planned": import.asset.as_str(),
                    "actual": hash.as_str(),
                }),
            ));
        }
    }
    // 2) 提交片段（与摆放，如果有）—— 唯一会改工程文档的那一步。
    if !import.ops.is_empty() {
        let now_ms = domain.now_ms();
        {
            let super::Domain {
                active,
                graph,
                undo,
                ..
            } = domain;
            let active = active.as_mut().ok_or_else(super::no_active_project)?;
            super::undo_session::commit(
                graph,
                &mut active.project,
                undo,
                super::CommitRequest {
                    now_ms,
                    // `McpEdit` = "MCP 代理的直接编辑"：本工具在活跃工程上直接落一条
                    // 片段（以及它的摆放），**不**创建提案 ⇒ `McpProposal` 不适用。
                    // （此前借的 `Import` 描述的是"外部工程/格式导入"，不是这件事。）
                    origin: OpOrigin::McpEdit {
                        agent_name: super::AGENT_NAME.to_owned(),
                    },
                    message: format!("import_audio {} ({})", import.clip.name, import.source_ref),
                    ops: import.ops.clone(),
                },
            )
            .map_err(super::undo_refusal_to_fault)?;
        }
        // 预测必须等于真实。
        let project = domain
            .active_project()
            .ok_or_else(super::no_active_project)?;
        let actual = digest_of(project)?;
        if import.digest_after.as_deref() != Some(actual.as_str()) {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                "音频登记后的工程摘要与只读规划的预测不一致 (预览会撒谎)",
                serde_json::json!({
                    "predicted": import.digest_after.clone(),
                    "actual": actual,
                }),
            ));
        }
    }
    let mut data = import.data()?;
    if let Value::Object(map) = &mut data {
        map.insert("applied".to_owned(), Value::from(!import.ops.is_empty()));
        map.insert(
            "assetPoolSize".to_owned(),
            Value::from(domain.asset_count()),
        );
        map.insert("commitCount".to_owned(), Value::from(domain.commit_count()));
    }
    Ok(ToolResponse::success(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use yeban_model::TrackKind;
    use yeban_model::samples::filled_project;

    /// 样本里那条 `TrackKind::Audio` 音轨（摆放的落点）。
    fn audio_track(project: &YebanProjectV1) -> EntityId {
        *project
            .tracks
            .iter()
            .find(|(_, track)| track.kind == TrackKind::Audio)
            .expect("规范样本里有一条音频轨")
            .0
    }

    /// 一份最小的 16-bit 单声道 WAV（判据自己构造，不依赖任何样本文件）。
    fn wav_s16(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2_u16.to_le_bytes());
        out.extend_from_slice(&16_u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    fn pool_of(bytes: &[u8]) -> BTreeMap<AssetHash, Vec<u8>> {
        BTreeMap::from([(AssetHash::of_bytes(bytes), bytes.to_vec())])
    }

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().expect("对象").clone()
    }

    /// 在克隆体上按**顺序**施加计划里的全部 `Op`。
    ///
    /// 与 [`super::super::undo_session::commit`] 的行为同一口径：那里把 `ops` 包成
    /// `Op::Batch`，而模型层的 `Batch` 是顺序施加 + 整体原子（`ops.rs` 的 `commit`）。
    fn apply_ops(project: &YebanProjectV1, import: &AudioImport) -> YebanProjectV1 {
        let mut after = project.clone();
        for op in &import.ops {
            op.apply(&mut after).expect("施加");
        }
        after
    }

    /// 按**逆序**施加全部逆操作（`Batch` 的逆也是逆序）。
    fn apply_inverse_ops(project: &mut YebanProjectV1, import: &AudioImport) {
        for op in import.ops.iter().rev() {
            op.apply_inverse(project).expect("撤销");
        }
    }

    #[test]
    fn asset_pool_source_records_the_measured_facts() {
        let project = filled_project();
        let bytes = wav_s16(48_000, &[0, 1_000, -1_000, 0]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let import = plan(
            &project,
            &pool,
            &args(&serde_json::json!({"name": "Kick", "assetHash": hash.as_str()})),
        )
        .expect("规划");
        assert_eq!(import.facts.sample_rate, 48_000);
        assert_eq!(import.facts.channels, 1);
        assert_eq!(import.facts.frames, 4);
        assert_eq!(import.facts.pcm_format, "s16");
        assert_eq!(import.facts.model_bit_depth.as_deref(), Some("Int16"));
        assert!(import.bytes.is_none(), "池里已有字节 ⇒ 不重复登记");
        assert!(import.clip_created(), "池里没有这个片段 ⇒ 要登记");
        assert!(
            import.placement.is_none(),
            "不给 `trackId` ⇒ 一位都不摆放 (与从前逐字节相同)"
        );
        assert!(!import.unchanged(), "有 op 就不是幂等命中");
        assert!(!import.declared_in_index);
        assert_eq!(import.asset, hash, "内容寻址: 计划里的资产就是池里的哈希");
        assert_eq!(import.source, ImportSource::AssetPool);
    }

    #[test]
    fn disk_source_decodes_and_registers_the_container_bytes() {
        let project = filled_project();
        let dir = std::env::temp_dir().join(format!("yeban-import-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("目录");
        let bytes = wav_s16(44_100, &[0, 2_000, -2_000, 0, 500, -500]);
        let path = dir.join("kick.wav");
        std::fs::write(&path, &bytes).expect("写");
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let import = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "path": path.display().to_string(),
                "gainDb": -1.5,
            })),
        )
        .expect("规划");
        assert_eq!(import.source, ImportSource::DiskPath);
        assert_eq!(import.facts.sample_rate, 44_100);
        assert_eq!(import.facts.frames, 6);
        assert_eq!(import.bytes.as_deref(), Some(bytes.as_slice()));
        assert_eq!(import.asset, AssetHash::of_bytes(&bytes));
        match &import.clip.content {
            ClipContent::Audio { gain_db, .. } => assert_eq!(*gain_db, -1.5),
            ClipContent::Midi { .. } => panic!("必须是音频片段"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_asset_is_entity_not_found_and_bad_hashes_are_parameter_errors() {
        let project = filled_project();
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": AssetHash::of_bytes(b"nope").as_str(),
            })),
        )
        .expect_err("池里没有");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "assetNotInPool");

        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({"name": "Kick", "assetHash": "not-a-hash"})),
        )
        .expect_err("坏哈希");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
    }

    #[test]
    fn source_must_be_exactly_one_of_two() {
        let project = filled_project();
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let fault = plan(&project, &pool, &args(&serde_json::json!({"name": "Kick"})))
            .expect_err("两个都没给");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            fault.into_result().expect("带内")["error"]["data"]["reason"],
            "sourceMissing"
        );

        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": "00".repeat(32),
                "path": "/tmp/x.wav",
            })),
        )
        .expect_err("两个都给了");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            fault.into_result().expect("带内")["error"]["data"]["reason"],
            "sourceAmbiguous"
        );
    }

    #[test]
    fn missing_file_is_file_not_found() {
        let project = filled_project();
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "path": "/definitely/not/here/kick.wav",
            })),
        )
        .expect_err("文件不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::FileNotFound));
    }

    #[test]
    fn a_non_audio_file_is_a_decode_failure_with_a_classification() {
        let project = filled_project();
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let dir = std::env::temp_dir().join(format!("yeban-import-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("目录");
        let path = dir.join("not-audio.bin");
        std::fs::write(&path, b"this is not audio at all").expect("写");
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "path": path.display().to_string(),
            })),
        )
        .expect_err("不是音频");
        assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
        let value = fault.into_result().expect("带内");
        assert!(value["error"]["data"]["decodeError"].is_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repeated_import_is_idempotent_by_content_and_the_inverse_is_exact() {
        let project = filled_project();
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let arguments = args(&serde_json::json!({
            "name": "Kick",
            "assetHash": hash.as_str(),
            "gainDb": -1.5,
        }));
        let first = plan(&project, &pool, &arguments).expect("规划");
        let mut after = apply_ops(&project, &first);
        let second = plan(&after, &pool, &arguments).expect("再规划");
        assert!(second.unchanged(), "内容相同 ⇒ 一位都不改");
        assert!(second.ops.is_empty(), "幂等命中 ⇒ 一条 Op 都不产出");
        assert!(second.digest_after.is_none(), "一位都不改 ⇒ 没有预测摘要");
        assert_eq!(
            first.clip.id, second.clip.id,
            "同一 (来源, 名字, 增益) ⇒ 同一片段身份"
        );
        // 换增益 ⇒ 换身份（增益是片段意图的一部分）。
        let louder = args(&serde_json::json!({
            "name": "Kick",
            "assetHash": hash.as_str(),
            "gainDb": 3.0,
        }));
        assert_ne!(
            plan(&after, &pool, &louder).expect("规划").clip.id,
            second.clip.id
        );
        // 逆操作逐字节回退 clip_pool。
        apply_inverse_ops(&mut after, &first);
        assert_eq!(after, project, "逆操作必须逐字节回退");
    }

    #[test]
    fn same_identity_with_different_content_is_a_conflict() {
        let project = filled_project();
        // ⚠ 必须是**真音频**：`plan` 会真的解码池里的字节（源若不是音频 ⇒ RENDER_FAILED，
        // 那条路径由 `a_non_audio_file_is_a_decode_failure_with_a_classification` 覆盖）。
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let first = plan(
            &project,
            &pool,
            &args(&serde_json::json!({"name": "Kick", "assetHash": hash.as_str()})),
        )
        .expect("规划");
        let after = apply_ops(&project, &first);
        // 同一个显式 clipId, 但换一个名字 ⇒ 同身份不同内容 ⇒ CONFLICT。
        let fault = plan(
            &after,
            &pool,
            &args(&serde_json::json!({
                "name": "Snare",
                "assetHash": hash.as_str(),
                "clipId": first.clip.id.to_canonical_string(),
            })),
        )
        .expect_err("冲突");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
    }

    #[test]
    fn budget_is_reported_from_the_single_source() {
        let value = budget_value();
        let budget = DecodeOptions::default().budget;
        assert_eq!(value["maxInputBytes"], budget.max_input_bytes);
        assert_eq!(value["maxPcmBytes"], budget.max_pcm_bytes);
        assert_eq!(value["maxChannels"], budget.max_channels);
        assert_eq!(value["maxSampleRate"], budget.max_sample_rate);
        assert_eq!(value["maxDurationSecs"], budget.max_duration_secs);
        assert_eq!(
            value["interleavedSamplesLimit"],
            budget.interleaved_samples_limit()
        );
    }

    // -----------------------------------------------------------------------
    // 摆放（`trackId` 一给就摆放）：`Op::AddClipPlacement` 那一半
    // -----------------------------------------------------------------------

    /// 摆放真的落在目标音轨上，时值缺省 = 素材全长换算，逆操作逐字节回退。
    #[test]
    fn a_placement_lands_on_the_track_and_reverses_byte_for_byte() {
        let project = filled_project();
        let track = audio_track(&project);
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]); // 4 帧（很短，但换算关系可断言）
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let import = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": hash.as_str(),
                "trackId": track.to_canonical_string(),
                "startTick": 960,
            })),
        )
        .expect("规划");
        assert!(import.clip_created(), "片段是新的 ⇒ 要登记");
        assert!(import.placement_created(), "摆放是新的 ⇒ 要摆放");
        assert_eq!(import.ops.len(), 2, "两条 Op: AddClip + AddClipPlacement");
        assert_eq!(
            import.ops[0].name(),
            "AddClip",
            "AddClip 必须在 AddClipPlacement 之前 (后者要求片段已在池里)"
        );
        assert_eq!(import.ops[1].name(), "AddClipPlacement");
        assert_eq!(import.placement_track, Some(track));
        let placement = import.placement.as_ref().expect("有摆放");
        assert_eq!(placement.clip_id, import.clip.id);
        assert_eq!(placement.start_tick, 960);
        assert!(!placement.muted);
        assert!(!placement.loop_config.enabled, "缺省不循环");
        assert!(import.placement_duration_derived, "时值来自素材全长");
        // 4 帧 @48k = 1/12000 s；128 BPM / 960 PPQ ⇒ 每秒 2048 tick ⇒ 0.1707 tick ⇒ 向上取整 1。
        assert_eq!(placement.duration_ticks, 1, "不足一 tick 也要给一整 tick");

        let after = apply_ops(&project, &import);
        let landed = after
            .tracks
            .get(&track)
            .expect("音轨还在")
            .clips
            .get(&placement.id)
            .expect("摆放必须落在这条音轨的 clips 上");
        assert_eq!(landed, placement, "落地的摆放必须逐字段等于计划");
        assert!(after.clip_pool.contains_key(&import.clip.id));

        let mut undone = after;
        apply_inverse_ops(&mut undone, &import);
        assert_eq!(
            undone, project,
            "逆操作 (RemoveClipPlacement → RemoveClip) 必须逐字节回退"
        );
    }

    /// 时值缺省 = `ceil(frames * PPQ * bpm / (sampleRate * 60))`（这里 4800 帧 @128 BPM）。
    #[test]
    fn the_default_duration_covers_the_whole_asset() {
        let project = filled_project();
        let track = audio_track(&project);
        let samples: Vec<i16> = (0..4_800)
            .map(|i| i16::try_from(i % 100).expect("小"))
            .collect();
        let bytes = wav_s16(48_000, &samples);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let import = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": hash.as_str(),
                "trackId": track.to_canonical_string(),
            })),
        )
        .expect("规划");
        // 4800 帧 @48k = 0.1 s；128 BPM / 960 PPQ ⇒ 0.1 * 2048 = 204.8 ⇒ ceil = 205。
        assert_eq!(
            import.placement.as_ref().expect("有摆放").duration_ticks,
            205
        );
        assert_eq!(import.facts.frames, 4_800);
    }

    /// 不给 `trackId` 就不许给其余四个：静默忽略会让调用方以为片段已经落轨。
    #[test]
    fn placement_siblings_without_a_track_are_rejected_not_ignored() {
        let project = filled_project();
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        for (field, value) in [
            ("startTick", serde_json::json!(10)),
            ("durationTicks", serde_json::json!(960)),
            (
                "placementId",
                serde_json::json!("01J8ZQ00000000000000000099"),
            ),
            ("muted", serde_json::json!(true)),
        ] {
            let mut arguments = serde_json::json!({"name": "Kick", "assetHash": hash.as_str()});
            arguments[field] = value;
            let fault = plan(&project, &pool, &args(&arguments)).expect_err("没有 trackId");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{field}"
            );
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "placementWithoutTrack");
            assert_eq!(value["error"]["data"]["field"], field);
        }
    }

    /// 显式时值 / `muted` 生效；同一位置重放幂等；同一身份换内容 ⇒ `CONFLICT`。
    #[test]
    fn explicit_duration_is_honoured_and_a_changed_placement_is_a_conflict() {
        let project = filled_project();
        let track = audio_track(&project);
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let arguments = args(&serde_json::json!({
            "name": "Kick",
            "assetHash": hash.as_str(),
            "trackId": track.to_canonical_string(),
            "startTick": 0,
            "durationTicks": 960,
            "muted": true,
        }));
        let first = plan(&project, &pool, &arguments).expect("规划");
        let placement = first.placement.expect("有摆放");
        assert_eq!(placement.duration_ticks, 960);
        assert!(placement.muted);
        assert!(!first.placement_duration_derived, "时值是显式给的");

        let after = apply_ops(&project, &first);
        // 逐字段相同的重放 ⇒ 一位都不改（片段与摆放都已存在）。
        let replay = plan(&after, &pool, &arguments).expect("重放");
        assert!(replay.unchanged(), "同一意图重放必须幂等: {:?}", replay.ops);
        // 幂等重放**不得**谎报一次摆放提交（`notes` 是三态而不是"要求了就写提交"）。
        let replay_notes = replay.data().expect("data")["notes"].clone();
        assert!(
            replay_notes
                .as_array()
                .expect("notes")
                .iter()
                .any(|note| note
                    .as_str()
                    .is_some_and(|text| text.contains("未重复提交"))),
            "幂等重放必须如实说明没有新提交: {replay_notes}"
        );

        // 同一 (片段, 音轨, 起点) 换时值 ⇒ 同身份不同内容 ⇒ CONFLICT。
        let mut changed = arguments.clone();
        changed.insert("durationTicks".to_owned(), Value::from(480));
        let fault = plan(&after, &pool, &changed).expect_err("同一摆放身份换内容");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["placementId"],
            placement.id.to_canonical_string()
        );

        // 换起点 ⇒ 换摆放身份 ⇒ 只补一条摆放（片段不再重复登记）。
        let mut moved = arguments.clone();
        moved.insert("startTick".to_owned(), Value::from(960));
        let second = plan(&after, &pool, &moved).expect("换位置");
        assert!(!second.clip_created(), "片段已存在 ⇒ 不重复登记");
        assert!(second.placement_created(), "新位置 ⇒ 新摆放");
        assert_eq!(second.ops.len(), 1, "只补摆放那一条 Op");
        assert_eq!(second.ops[0].name(), "AddClipPlacement");
    }

    /// 目标音轨不存在 ⇒ `TRACK_NOT_FOUND`（不是静默建一条）。
    #[test]
    fn a_placement_on_a_missing_track_is_track_not_found() {
        let project = filled_project();
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let ghost = EntityId::new().to_canonical_string();
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": hash.as_str(),
                "trackId": ghost,
            })),
        )
        .expect_err("音轨不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
    }

    /// 零时值 / 非法时值类型都被明确拒绝（模型层也拒绝零时值，这里必须更早）。
    #[test]
    fn a_zero_duration_placement_is_rejected_before_the_model() {
        let project = filled_project();
        let track = audio_track(&project);
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": hash.as_str(),
                "trackId": track.to_canonical_string(),
                "durationTicks": 0,
            })),
        )
        .expect_err("零时值");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["field"], "durationTicks");
    }

    /// 响应的 `data.placement`：不给 `trackId` 时是 `null`，给了就是逐字段的事实。
    #[test]
    fn the_response_reports_the_placement_and_its_duration_rule() {
        let project = filled_project();
        let track = audio_track(&project);
        let bytes = wav_s16(48_000, &[0, 1, 2, 3]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);

        let without = plan(
            &project,
            &pool,
            &args(&serde_json::json!({"name": "Kick", "assetHash": hash.as_str()})),
        )
        .expect("规划");
        let data = without.data().expect("data");
        assert_eq!(data["placement"], Value::Null, "没要求摆放 ⇒ null");
        assert_eq!(data["created"], true);
        assert_eq!(data["unchanged"], false);
        assert!(
            data["notes"]
                .as_array()
                .expect("notes")
                .iter()
                .any(|note| note
                    .as_str()
                    .is_some_and(|text| text.contains("没有要求摆放"))),
            "必须如实说明片段不在任何轨道上: {data}"
        );

        let with = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Kick",
                "assetHash": hash.as_str(),
                "trackId": track.to_canonical_string(),
                "startTick": 1920,
                "durationTicks": 3840,
            })),
        )
        .expect("规划");
        let data = with.data().expect("data");
        assert_eq!(data["placement"]["trackId"], track.to_canonical_string());
        assert_eq!(data["placement"]["startTick"], 1920);
        assert_eq!(data["placement"]["durationTicks"], 3840);
        assert_eq!(data["placement"]["muted"], false);
        assert_eq!(
            data["placement"]["placed"], true,
            "本次真的会摆放 (dryRun 预览与真做共用这一个函数)"
        );
        assert!(
            data["placement"]["durationRule"]
                .as_str()
                .is_some_and(|text| text.contains("显式")),
            "{data}"
        );
        assert!(
            data["notes"]
                .as_array()
                .expect("notes")
                .iter()
                .any(|note| note
                    .as_str()
                    .is_some_and(|text| text.contains("Op::AddClipPlacement"))),
            "必须说明片段真的落轨了: {data}"
        );
    }
    /// `gainDb` 的非有限闸门判在 **f64 → f32 收窄之后**，且与别的闸门分得开。
    ///
    /// 第二轮注入实测：`IA-gainfinite`（`if !value.is_finite()` → `if false`）
    /// **全绿**。机械前提：`serde_json` 在**文本层**就拒绝越界浮点（`"1e400"` 是解析期
    /// 错误），但 **`1e39` 在 f64 里有限**、收窄到 `f32` 之后是 `inf` ⇒ 这一道闸门
    /// 是活的（与 `ef459f9` 修掉的是同一类缺陷：`setParam` 的有限性判在收窄之后）。
    #[test]
    fn a_gain_that_only_overflows_after_narrowing_is_refused_as_non_finite() {
        let project = filled_project();
        let bytes = wav_s16(48_000, &[0, 1_000, -1_000, 0]);
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let fault = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Loud",
                "assetHash": hash.as_str(),
                "gainDb": 1e39,
            })),
        )
        .expect_err("1e39 收窄到 f32 之后是 inf, 必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let Fault::Domain { data, .. } = &fault else {
            panic!("应当是领域失败, 实际 {fault:?}");
        };
        let data = data.as_ref().expect("本形态的失败必须带 data");
        assert_eq!(data["field"], "gainDb");
        assert_eq!(data["reason"], "nonFiniteValue", "{data}");
        // 阴性对照: 正常增益照旧放行（上面红的不是"什么都拒"）。
        let ok = plan(
            &project,
            &pool,
            &args(&serde_json::json!({
                "name": "Loud",
                "assetHash": hash.as_str(),
                "gainDb": -1.5,
            })),
        )
        .expect("普通增益必须放行");
        assert!(!ok.ops.is_empty());
    }
}
