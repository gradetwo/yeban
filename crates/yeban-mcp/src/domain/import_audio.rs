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
use yeban_model::{AssetHash, ClipContent, ClipPoolEntry, EntityId, Op, OpOrigin, YebanProjectV1};

use super::error::{self, Fault};
use super::extension_pure::{self, ImportSource};
use super::ids::deterministic_id;
use super::render::AssetStore;
use crate::tools::{ErrorCode, ToolResponse};

/// 一次音频导入的**完整只读规划**（`dryRun` 与真做共用）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioImport {
    /// 将要（或已经）登记进 `clip_pool` 的条目。
    pub clip: ClipPoolEntry,
    /// 将要提交的领域操作；`None` = 池里已有逐字段相同的条目（幂等命中，不改任何东西）。
    pub op: Option<Op>,
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

    /// 响应的 `data`（`dryRun` 预览与真做**共用这一个函数**）。
    ///
    /// # Errors
    ///
    /// 摘要序列化失败 → `IO_ERROR`。
    pub fn data(&self) -> Result<Value, Fault> {
        let (asset, gain_db) = self.asset_ref()?;
        Ok(serde_json::json!({
            "imported": true,
            "created": self.op.is_some(),
            "unchanged": self.op.is_none(),
            "clip": {
                "clipId": self.clip.id.to_canonical_string(),
                "name": self.clip.name.clone(),
                "asset": asset,
                "gainDb": gain_db,
                "contentKind": "Audio",
            },
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
            "notes": notes(self.bytes.is_some()),
        }))
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
fn notes(registered_bytes: bool) -> Vec<&'static str> {
    let mut notes = vec![
        "CAS 池的字节不是 Op 的载荷 (模型 Op 全集没有资产变体) ⇒ 撤销 clip_pool 条目不会回收池里的字节 (needs-2)",
        "工程 assets 索引 (许可/原路径/字节数) 同样没有 Op ⇒ 本工具只登记 clip_pool 条目, 不写索引 (needs-3)",
    ];
    if registered_bytes {
        notes.push("本次真的向会话 CAS 池登记了容器字节 (内容寻址: 同一份字节不会重复登记)");
    } else {
        notes.push("字节已在会话 CAS 池里 (按哈希引用, 未重复登记)");
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

    // ---- 幂等 / 冲突（按内容与意图，不靠幂等键） ----
    let mut op = None;
    let mut digest_after = None;
    match project.clip_pool.get(&clip_id) {
        Some(existing) if *existing == clip => {
            // 逐字段相同 ⇒ 一位都不改（幂等命中）。
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
        None => {
            let candidate = Op::AddClip { clip: clip.clone() };
            let mut simulated = project.clone();
            candidate
                .apply(&mut simulated)
                .map_err(|error| error::from_model("音频片段登记模拟", &error))?;
            simulated
                .validate()
                .map_err(|error| error::from_model("音频片段登记校验", &error))?;
            digest_after = Some(digest_of(&simulated)?);
            op = Some(candidate);
        }
    }

    Ok(AudioImport {
        clip,
        op,
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
            serde_json::json!({ "field": "gainDb", "value": value }),
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

/// **施加**一次音频导入：先登记 CAS 字节（内容寻址），再提交 `Op::AddClip`。
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
    // 2) 提交片段条目（唯一会改工程文档的那一步）。
    if let Some(op) = &import.op {
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
                    // `Import` 正是这一档: "外部工程/格式导入" —— 本工具的语义就是它。
                    origin: OpOrigin::Import,
                    message: format!("import_audio {} ({})", import.clip.name, import.source_ref),
                    ops: vec![op.clone()],
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
        map.insert("applied".to_owned(), Value::from(import.op.is_some()));
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

    use yeban_model::samples::filled_project;

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
        assert!(import.op.is_some(), "池里没有这个片段 ⇒ 要登记");
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
        let mut after = project.clone();
        first
            .op
            .as_ref()
            .expect("要登记")
            .apply(&mut after)
            .expect("施加");
        let second = plan(&after, &pool, &arguments).expect("再规划");
        assert!(second.op.is_none(), "内容相同 ⇒ 一位都不改");
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
        first
            .op
            .as_ref()
            .expect("要登记")
            .apply_inverse(&mut after)
            .expect("撤销");
        assert_eq!(after, project, "逆操作必须逐字节回退");
    }

    #[test]
    fn same_identity_with_different_content_is_a_conflict() {
        let project = filled_project();
        let bytes = b"a".to_vec();
        let hash = AssetHash::of_bytes(&bytes);
        let pool = pool_of(&bytes);
        let first = plan(
            &project,
            &pool,
            &args(&serde_json::json!({"name": "Kick", "assetHash": hash.as_str()})),
        )
        .expect("规划");
        let mut after = project.clone();
        first
            .op
            .as_ref()
            .expect("要登记")
            .apply(&mut after)
            .expect("施加");
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
}
