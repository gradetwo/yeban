//! ⭐ `ADR-0030`：`yeban_export_stems` —— **一次调用**把每条轨弹成一个 32-bit Float WAV。
//!
//! ## 它补的是哪个缺口（`G9b`）
//!
//! 工具面此前**只有母带**。想要"每条轨一个 WAV"，调用方必须自己完成
//! `N × (克隆工程 → 只留一条轨 → yeban_render_master → 换文件名)` 的往返，
//! 还要自己保证落点不覆盖、命名不漂移。渲染器**早就**能单轨出声（`track_contributions`
//! 就是这么做的），缺的只是"一次调用"这件事。本片把它做成一个扩展工具。
//!
//! ## 两阶段切分照 `yeban_render_master`
//!
//! [`export`] 是**纯只读**的：`N + 1` 次渲染全部在这里完成，产物是内存里的容器字节。
//! [`write`] 才是唯一的写盘处，且**逐条**写 —— 跨 N 个文件没有事务
//! （[`store::write_project_atomic`] 只保证**单文件**原子），所以第 k 条失败时
//! 前 k−1 条已经在盘上。这条事实决定了 D5 的响应形状：部分成功是**一等状态**。
//!
//! ⛔ 不回滚删除：删用户盘上的文件比留一个如实的 `orphanedFiles` 危险。
//!
//! ## 为什么 `N + 1` 次，不是 `N` 次（D3a）
//!
//! [`render::build`] 的帧数由**可闻**源的末端推导（`end_tick = max(report.end_tick)`）。
//! 逐轨渲染时"只留一条轨"必然缩短这个末端 ⇒ 各条分轨长度不齐，下游没法对齐。
//! 因此先做**一次**全工程基线渲染（试听状态已复位，见 D4）取 `frames`，
//! 再把这个帧数钉死进每一条分轨。多出来的那一次是真实成本，
//! 所以 [`render::build`] 的复用次数与 `stemsBasis.renders` 都报 `N + 1`。
//!
//! ## 为什么分轨是 32-bit Float（D2）
//!
//! 口径来自 `ARCH-DSP-003`（音轨冻结与母带分轨：**32-bit Float**），不是选型自由。
//! 容器侧由 [`render::write_stem_container`] 走 `PcmFormat::float(...)`，
//! `yeban-render` 的 `ContainerPlan` 对非 PCM 容器**自动**补 `fact` chunk
//! （⚠ 漏掉它就是个坏文件，见 `crates/yeban-render/src/rf64.rs` 的容器规则）。
//! 母带那条 24-bit + TPDF 的路径一位都不经过这里。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use serde_json::{Map, Value, json};
use yeban_model::{EntityId, YebanProjectV1};

use super::error::Fault;
use super::{render, store};
use crate::tools::ErrorCode;

/// 分轨文件的扩展名（⛔ 不提供 `format` 实参：分轨的下游是二次混音，容器只有 WAV 一种口径）。
pub const STEM_FILE_EXTENSION: &str = "wav";

/// 文件名序号的最小宽度（`0001.wav`）。轨数超过 4 位时按实际位数加宽。
pub const STEM_INDEX_MIN_WIDTH: usize = 4;

/// `stemsBasis.basis` 的固定措辞（成本读数必须**自己说清**代价，`WALL ⑦` 的先例）。
pub const STEMS_BASIS_TEXT: &str = "per-track re-render of the full project window (one project-wide basis render + one render per stem)";

/// 一次已校验的分轨导出请求（**还没有渲染**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StemRequest {
    /// 输出目录：**必须已存在**（⛔ 不 `create_dir_all`：分轨是写一堆文件，目录写错位置比单文件更贵）。
    pub out_dir: PathBuf,
    /// 选中的轨；`None` = 工程里**全部**非 master 轨（D4：批次内容不随试听状态漂移）。
    pub track_ids: Option<Vec<EntityId>>,
}

impl StemRequest {
    /// `dryRun` 与响应共用的请求回显。
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "outDir": self.out_dir.display().to_string(),
            "trackIds": self.track_ids.as_ref().map(|ids| {
                Value::Array(
                    ids.iter()
                        .map(|id| Value::from(id.to_canonical_string()))
                        .collect::<Vec<_>>(),
                )
            }),
        })
    }
}

/// 校验 `yeban_export_stems` 的实参（**纯函数**，不读盘、不碰工程）。
///
/// # Errors
///
/// - `INVALID_PARAMETER_RANGE`：`outDir` 缺失/空/非字符串/不存在/不是目录；
///   `trackIds` 不是字符串数组或含非法 ULID。
pub fn validate(arguments: &Map<String, Value>) -> Result<StemRequest, Fault> {
    let out_dir = match arguments.get("outDir") {
        Some(Value::String(text)) if !text.trim().is_empty() => PathBuf::from(text),
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`outDir` 必须是非空字符串 (分轨要写 N 个文件, 目录必须由调用方给定且已存在)",
                json!({ "field": "outDir", "received": other }),
            ));
        }
        None => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`outDir` 是必填实参: 分轨导出会写 N 个文件, 不接受缺省目录",
            ));
        }
    };
    if !out_dir.exists() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`outDir` 不存在: {}", out_dir.display()),
            json!({ "missing": out_dir.display().to_string(), "why": "分轨导出刻意不创建目录: 批量写文件时把目录建错位置比单个文件更贵" }),
        ));
    }
    if !out_dir.is_dir() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`outDir` 不是目录: {}", out_dir.display()),
            json!({ "notADirectory": out_dir.display().to_string() }),
        ));
    }
    let track_ids = match arguments.get("trackIds") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                let text = item.as_str().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        "`trackIds` 的每一项必须是 26 字符 ULID 字符串",
                        json!({ "received": item }),
                    )
                })?;
                let id = EntityId::from_str(text).map_err(|error| {
                    Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("`trackIds` 里有一项不是合法 ULID: {error}"),
                        json!({ "received": text }),
                    )
                })?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Some(ids)
        }
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`trackIds` 必须是字符串数组",
                json!({ "field": "trackIds", "received": other }),
            ));
        }
    };
    Ok(StemRequest { out_dir, track_ids })
}

/// 一条轨**不出文件**的原因 —— 封闭枚举，每支都在
/// `crates/yeban-mcp/tests/fault_vocabulary.rs` 的黄金表里登记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 主总线不是"一条轨的素材"（它就是母带本身）。
    MasterBusTrack,
    /// 该轨在路由图里不是任何源节点（总线 / 分组轨）⇒ 弹出来的不是"这条轨的内容"。
    NotASourceNode,
}

/// 一条**被登记但不出文件**的轨（D4：⛔ 不静默丢轨）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedTrack {
    /// 轨身份。
    pub track_id: EntityId,
    /// 轨名（原样，⛔ 不清洗 —— 清洗只作用于文件名）。
    pub name: String,
    /// 为什么不出文件。
    pub reason: SkipReason,
}

impl SkippedTrack {
    #[must_use]
    fn to_value(&self) -> Value {
        let track_id = self.track_id.to_canonical_string();
        // 字面量 `reason`（`crates/yeban-mcp/tests/fault_vocabulary.rs` 的冻结面只认字面站点）。
        match self.reason {
            SkipReason::MasterBusTrack => json!({
                "trackId": track_id,
                "name": self.name,
                "reason": "masterBusTrack",
            }),
            SkipReason::NotASourceNode => json!({
                "trackId": track_id,
                "name": self.name,
                "reason": "notASourceNode",
            }),
        }
    }
}

/// 一条**已渲染、尚未落盘**的分轨。
///
/// 刻意**不实现 `Clone`**：`artifact.bytes` 是几十万到几百万字节的容器负载，
/// 复制一条等于复制整段音频；`RenderArtifact` 本身也因此不 `Clone`。
#[derive(Debug)]
pub struct StemRow {
    /// 轨身份。
    pub track_id: EntityId,
    /// 轨名（工程里的原文）。
    pub name: String,
    /// 清洗后的文件名（`<index>.<sanitized>.wav`）。
    pub file_name: String,
    /// 绝对/给定的落点（`out_dir` + `file_name`）。
    pub path: PathBuf,
    /// 导出**当时**该轨的 `mute` 读数（回执必须让调用方看出"这条本来在工程里是静音的"）。
    pub source_mute: bool,
    /// 导出**当时**该轨的 `solo` 读数。
    pub source_solo: bool,
    /// 已编码的分轨容器。
    pub artifact: render::RenderArtifact,
}

impl StemRow {
    /// 逐轨回执（与母带的同名键**同形**，便于调用方复用同一套校验代码）。
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "trackId": self.track_id.to_canonical_string(),
            "name": self.name,
            "fileName": self.file_name,
            "path": self.path.display().to_string(),
            "sha256": self.artifact.sha256.clone(),
            "bytes": self.artifact.bytes.len(),
            "headerBytes": self.artifact.header_bytes,
            "payloadBytes": self.artifact.payload_bytes,
            "bitDepth": self.artifact.bit_depth,
            "channels": self.artifact.channels,
            "frames": self.artifact.frames,
            "container": self.artifact.container_name(),
            "sourceMute": self.source_mute,
            "sourceSolo": self.source_solo,
        })
    }

    /// `dryRun` 预览（**不**声称已落盘）。
    #[must_use]
    fn to_preview_value(&self) -> Value {
        let mut value = self
            .to_value()
            .as_object()
            .expect("to_value 产出对象")
            .clone();
        value.insert("wouldWrite".to_owned(), json!(true));
        value.insert(
            "bytesRenderedInMemory".to_owned(),
            json!(self.artifact.bytes.len()),
        );
        Value::Object(value)
    }
}

/// `export` 的产物：全部字节在内存里，只差落盘。
///
/// 与 [`StemRow`] 同理**不实现 `Clone`**：这批字节按"一次构造、一次落盘"的路线走，
/// 复制整批等于复制整张唱片的分轨素材。
#[derive(Debug)]
pub struct StemBatch {
    /// 已校验的请求（原样回显）。
    pub request: StemRequest,
    /// 逐轨产物，顺序 = 工程内轨顺序（`BTreeMap` 序 = 跨进程确定序 [MODEL-AST-003]）。
    pub stems: Vec<StemRow>,
    /// 点名但**不出文件**的轨（`masterBusTrack` / `notASourceNode`）—— D4 的"⛔ 不静默丢轨"。
    pub skipped: Vec<SkippedTrack>,
    /// [`render::build`] 的**实测**调用次数：基线那一次 + 每一条**出文件**的分轨各一次
    /// （判据钉 `renders == stems.len() + 1`，见 D3a）。
    ///
    /// 被跳过的总线轨**不**进这个数 —— 成本读数数"真的花了多少次"，⛔ 不数"点名了几条"。
    pub renders: usize,
    /// 基线渲染的**实测**帧数 = 每一条分轨被钉死的帧数（批次对齐的唯一来源）。
    pub basis_frames: u64,
    /// 本批的采样率（Hz）= **工程**采样率（⛔ 不新增 `sampleRate` 实参：分轨的口径是
    /// "不掉精度的素材"，重采样属于下游的事）。
    pub sample_rate: u32,
}

impl StemBatch {
    /// 本批产物的字节合计（单位：字节）。
    #[must_use]
    pub fn bytes_total(&self) -> usize {
        self.stems.iter().map(|row| row.artifact.bytes.len()).sum()
    }

    /// 最长的清洗后文件名的**UTF-8 字节数**（目录名由调用方控制，这里只报自己造的那一段）。
    #[must_use]
    pub fn max_file_name_bytes(&self) -> usize {
        self.stems
            .iter()
            .map(|row| row.file_name.len())
            .max()
            .unwrap_or(0)
    }

    /// 批次级成本读数（D6）。
    #[must_use]
    pub fn stems_basis(&self) -> Value {
        json!({
            "renders": self.renders,
            "basisRender": "projectWide",
            "audioFramesPerRender": self.basis_frames,
            "sampleRate": self.sample_rate,
            "bitDepth": render::STEM_BIT_DEPTH,
            "basis": STEMS_BASIS_TEXT,
        })
    }

    /// `dryRun` 预览：全部实测数字，一个字节都不写盘。
    #[must_use]
    pub fn preview(&self) -> Value {
        let mut map = Map::new();
        map.insert("stems".to_owned(), self.preview_stems());
        map.insert("skippedTracks".to_owned(), self.preview_skipped());
        map.insert("stemsBasis".to_owned(), self.stems_basis());
        map.insert("bytesTotal".to_owned(), json!(self.bytes_total()));
        map.insert(
            "maxFileNameBytes".to_owned(),
            json!(self.max_file_name_bytes()),
        );
        map.insert("auditionStateIgnored".to_owned(), json!(true));
        map.insert("bitDepth".to_owned(), json!(render::STEM_BIT_DEPTH));
        map.insert(
            "outputDir".to_owned(),
            json!(self.request.out_dir.display().to_string()),
        );
        map.insert("request".to_owned(), self.request.to_value());
        map.insert("willWrite".to_owned(), json!(true));
        map.insert("written".to_owned(), json!(false));
        Value::Object(map)
    }

    /// 成功落盘后的响应 `data`（D5：`partial:false`）。
    #[must_use]
    pub fn response_data(&self, completed: &[StemRow]) -> Value {
        let mut map = Map::new();
        map.insert(
            "stems".to_owned(),
            Value::Array(completed.iter().map(StemRow::to_value).collect()),
        );
        map.insert("partial".to_owned(), json!(false));
        map.insert("stemCount".to_owned(), json!(completed.len()));
        map.insert("skippedTracks".to_owned(), self.preview_skipped());
        map.insert("stemsBasis".to_owned(), self.stems_basis());
        map.insert(
            "bytesTotal".to_owned(),
            json!(
                completed
                    .iter()
                    .map(|row| row.artifact.bytes.len())
                    .sum::<usize>()
            ),
        );
        map.insert(
            "maxFileNameBytes".to_owned(),
            json!(self.max_file_name_bytes()),
        );
        map.insert("auditionStateIgnored".to_owned(), json!(true));
        map.insert(
            "outputDir".to_owned(),
            json!(self.request.out_dir.display().to_string()),
        );
        map.insert("request".to_owned(), self.request.to_value());
        Value::Object(map)
    }

    fn preview_stems(&self) -> Value {
        Value::Array(
            self.stems
                .iter()
                .map(StemRow::to_preview_value)
                .collect::<Vec<_>>(),
        )
    }

    fn preview_skipped(&self) -> Value {
        Value::Array(self.skipped.iter().map(SkippedTrack::to_value).collect())
    }
}

/// ⭐ 渲染整个批次（**只读**：`N + 1` 次 [`render::build`]，零字节落盘）。
///
/// # Errors
///
/// - `INVALID_PARAMETER_RANGE`：`outDir` 与工程文件/锁文件同路径，或某个既有目标文件已被占用；
/// - `TRACK_NOT_FOUND`：`trackIds` 里的身份不在工程里；
/// - `RENDER_FAILED`：渲染层拒绝（含"工程里没有可渲染的内容"）。
#[allow(clippy::too_many_lines)]
pub fn export(
    project: &YebanProjectV1,
    project_path: &Path,
    request: &StemRequest,
    now_ms: u64,
    assets: &dyn render::AssetStore,
) -> Result<StemBatch, Fault> {
    // 0. `trackIds` 里的陌生身份 ⇒ 响亮拒绝（⛔ 不静默忽略拼错的 ULID，`G1` 的教训）。
    if let Some(ids) = &request.track_ids {
        let unknown = ids
            .iter()
            .filter(|id| !project.tracks.contains_key(id))
            .map(|id| id.to_canonical_string())
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(Fault::domain_with_data(
                ErrorCode::TrackNotFound,
                "`trackIds` 里有工程中不存在的音轨",
                json!({
                    "unknownTrackIds": unknown,
                    "knownTrackIds": project
                        .tracks
                        .keys()
                        .map(|id| id.to_canonical_string())
                        .collect::<Vec<_>>(),
                }),
            ));
        }
    }
    // 1. 选轨 + 序号：`tracks` 是 `BTreeMap` ⇒ 顺序是身份的确定序 [MODEL-AST-003]。
    //    master 总线不是"一条轨"（分轨的产物喂回总线没有意义），因此不占序号。
    let selected = request
        .track_ids
        .as_ref()
        .map(|ids| ids.iter().copied().collect::<BTreeSet<_>>());
    let mut queue: Vec<(usize, EntityId, &str)> = Vec::with_capacity(project.tracks.len());
    let mut skipped: Vec<SkippedTrack> = Vec::new();
    let mut next_index = 1_usize;
    for (&track_id, track) in &project.tracks {
        if track_id == project.master_bus_track_id {
            // 主总线不是"一条轨的素材"（它就是母带本身，`yeban_render_master` 那条工具）。
            // ⛔ 不静默消失：它在 `skippedTracks` 里点名（D4）。
            skipped.push(SkippedTrack {
                track_id,
                name: track.name.clone(),
                reason: SkipReason::MasterBusTrack,
            });
            continue;
        }
        let index = next_index;
        next_index = next_index.saturating_add(1);
        if selected
            .as_ref()
            .is_some_and(|set| !set.contains(&track_id))
        {
            continue;
        }
        queue.push((index, track_id, track.name.as_str()));
    }
    // 2. 基线渲染：全工程可闻（试听状态已复位），帧数**由时间轴推导**。
    //    ⛔ 不自己算"工程有多长"：那条公式只有 `build` 一处实现（D3a）。
    let basis_project = project_without_audition(project);
    let basis_request = stem_render_request(&basis_project, None, None);
    let basis = render::build(&basis_project, project_path, &basis_request, now_ms, assets)?;
    let source_nodes = basis.source_nodes.iter().copied().collect::<BTreeSet<_>>();
    // 3. 逐轨渲染。每个落点先过 `guard_output_path`（工程文件 / 锁文件），
    //    再加一条**批内预检**：目标已存在就整批拒绝（`write_project_atomic` 会覆盖它）。
    let mut stems = Vec::with_capacity(queue.len());
    let mut renders = 1_usize;
    for (index, track_id, name) in queue {
        if !source_nodes.contains(&track_id) {
            // 总线/分组轨：它自己的信号来自子轨，逐轨弹出来的东西不是"这条轨的内容"。
            skipped.push(SkippedTrack {
                track_id,
                name: name.to_owned(),
                reason: SkipReason::NotASourceNode,
            });
            continue;
        }
        let file_name = stem_file_name(index, project.tracks.len(), name);
        let path = request.out_dir.join(&file_name);
        render::guard_output_path(&path, project_path)?;
        if path.exists() {
            return Err(collisions_fault(&[&path], "目标文件已存在"));
        }
        let mut stem_project = project_without_audition(project);
        // 只留这一条源节点可闻：其余源节点**在克隆上**静音（D3 的克隆-改-重渲染）。
        // 总线与 master 不动 —— 它们是这一条信号的通路，静音它们会连目标一起消掉。
        for &node in &source_nodes {
            if node == track_id {
                continue;
            }
            if let Some(track) = stem_project.tracks.get_mut(&node) {
                track.mute = true;
            }
        }
        let stem_request = stem_render_request(&stem_project, Some(basis.frames), Some(path));
        let artifact = render::build(&stem_project, project_path, &stem_request, now_ms, assets)?;
        renders = renders.saturating_add(1);
        let source = project
            .tracks
            .get(&track_id)
            .expect("轨来自同一份 tracks 遍历");
        stems.push(StemRow {
            track_id,
            name: name.to_owned(),
            file_name,
            path: artifact.path.clone(),
            source_mute: source.mute,
            source_solo: source.solo,
            artifact,
        });
    }
    // ⛔ 不做隐式 ` (1)` 去重：文件名前缀的序号在**全部**非 master 轨上唯一，
    //    两条不同轨不可能撞同一个落点（清洗只改字符、不改序号）。
    Ok(StemBatch {
        request: request.clone(),
        stems,
        skipped,
        renders,
        basis_frames: basis.frames,
        sample_rate: project.sample_rate().hz(),
    })
}

/// ⭐ 逐条原子落盘（唯一的副作用点，D5）。
///
/// 第 k 条失败时：前 k−1 条**保留不删**，回一条 `IO_ERROR` + `errorKind:"stemWriteFailed"`，
/// `data` 里给出 `completedStems` / `failedTrackId` / `failedReason` / `orphanedFiles` /
/// `resumeFrom`。⛔ 不用 `"reason"` 这个键名 —— 它是故障词汇表冻结面的字面量，新增一处
/// 就要重钉一次金标（见 `docs/DEVELOPMENT_LEDGER.md` 的 `fault_vocabulary` 条目）。
pub fn write(batch: &StemBatch) -> Result<Value, Fault> {
    for (position, row) in batch.stems.iter().enumerate() {
        if let Err(fault) = store::write_project_atomic(&row.path, &row.artifact.bytes) {
            return Err(write_failed(batch, &batch.stems[..position], row, &fault));
        }
    }
    Ok(batch.response_data(&batch.stems))
}

/// 把落盘失败包成 D5 形状的部分成功回执。
fn write_failed(
    batch: &StemBatch,
    completed: &[StemRow],
    failed: &StemRow,
    fault: &Fault,
) -> Fault {
    let (failed_reason, inner_data) = match fault {
        Fault::Domain { message, data, .. } => (message.clone(), data.clone()),
        Fault::Impl { error } => (error.message.clone(), None),
    };
    // `orphanedFiles`：**实测**盘上存在但本批未完成的路径（含被中断的目标残留字节）。
    let orphaned = batch
        .stems
        .iter()
        .filter(|row| !completed.iter().any(|done| done.track_id == row.track_id))
        .map(|row| row.path.clone())
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    let remaining = batch
        .stems
        .iter()
        .skip_while(|row| row.track_id != failed.track_id)
        .map(|row| row.track_id.to_canonical_string())
        .collect::<Vec<_>>();
    let mut data = Map::new();
    data.insert("errorKind".to_owned(), json!("stemWriteFailed"));
    data.insert(
        "failedTrackId".to_owned(),
        json!(failed.track_id.to_canonical_string()),
    );
    data.insert("failedFileName".to_owned(), json!(failed.file_name.clone()));
    data.insert(
        "failedPath".to_owned(),
        json!(failed.path.display().to_string()),
    );
    data.insert("failedReason".to_owned(), json!(failed_reason));
    if let Some(inner) = inner_data {
        data.insert("failedData".to_owned(), inner);
    }
    data.insert(
        "completedStems".to_owned(),
        Value::Array(completed.iter().map(StemRow::to_value).collect()),
    );
    data.insert(
        "orphanedFiles".to_owned(),
        Value::Array(
            orphaned
                .iter()
                .map(|path| json!(path.display().to_string()))
                .collect::<Vec<_>>(),
        ),
    );
    data.insert(
        "resumeFrom".to_owned(),
        json!({
            "outDir": batch.request.out_dir.display().to_string(),
            "trackIds": remaining,
        }),
    );
    data.insert("partial".to_owned(), json!(true));
    data.insert("auditionStateIgnored".to_owned(), json!(true));
    data.insert("stemsBasis".to_owned(), batch.stems_basis());
    data.insert(
        "writtenBeforeFailure".to_owned(),
        json!(
            completed
                .iter()
                .map(|row| row.artifact.bytes.len())
                .sum::<usize>()
        ),
    );
    data.insert(
        "rollback".to_owned(),
        json!("none: 已落盘的分轨不会被删除 (删用户盘上的文件比留一个如实的 orphanedFiles 危险)"),
    );
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!(
            "分轨落盘在第 {} 条失败: {} (前 {} 条已保留在盘上)",
            completed.len() + 1,
            failed.file_name,
            completed.len()
        ),
        Value::Object(data),
    )
}

/// 落点冲突的结构化拒绝（预检阶段，还没写任何字节）。
fn collisions_fault(paths: &[&PathBuf], why: &str) -> Fault {
    Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!("分轨落点冲突: {why} (⛔ 不做隐式改名, 也不覆盖既有文件)"),
        json!({
            "conflictingPaths": paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
            "why": why,
            "hint": "换一个已存在的空目录 (outDir), 或先移走这些文件",
        }),
    )
}

/// 把工程里的**试听状态**复位成"全部可闻"（D4）。
///
/// `track_is_audible(track, any_solo)` 的语义是 `!mute && (!any_solo || solo || solo_safe)`：
/// 工程里只要有任何 `solo`，非 `solo_safe` 的轨就不发声 ⇒ 照搬这条门，批次内容会随
/// 用户上一次按了哪个独听键而漂移。分轨是给下游用的素材，不是试听快照。
///
/// 同时按 D3 摘掉**母带链上的插入**：分轨是轨后输出，不是总线快照。
fn project_without_audition(project: &YebanProjectV1) -> YebanProjectV1 {
    let mut clone = project.clone();
    for track in clone.tracks.values_mut() {
        track.mute = false;
        track.solo = false;
        track.solo_safe = false;
    }
    if let Some(master) = clone.tracks.get_mut(&clone.master_bus_track_id) {
        master.devices.clear();
    }
    clone
}

/// 构造一次分轨渲染请求：32-bit Float、**不归一化**、帧数钉死、逐轨不同落点。
///
/// ⚠ `normalize` 恒 `false` 是有意的：归一化是**母带**动作（整批共用一个增益），
/// 逐轨各归一次会让各条之间的电平关系失真，下游就没法混了。
fn stem_render_request(
    project: &YebanProjectV1,
    frames: Option<u64>,
    path: Option<PathBuf>,
) -> render::RenderRequest {
    render::RenderRequest {
        // 分轨只有 WAV 口径（`STEM_FILE_EXTENSION`）；超过 4 GiB 时 `yeban-render`
        // 自己把容器升级成 RF64，`container` 字段如实回报。
        format_index: 0,
        sample_rate: project.sample_rate(),
        normalize: false,
        frames,
        duration_seconds_bits: None,
        target_lufs_bits: None,
        path,
        contributions: false,
        // ⭐ 审评 2026-10-31（P1-1）：分轨工具没有 threads 实参 ⇒ 基线走缺省单线程
        //（本函数不接调用方请求，`request` 也不在作用域里，⛔ 不虚构继承）。
        threads: None,
        sample_format: render::SampleFormat::Float32,
    }
}

/// 确定性清洗轨名 → 文件名（D7）。
///
/// 规则：保留 `[0-9A-Za-z._-]`，其余**逐个**换成 `_`；去掉首尾的 `.`/`_`；
/// 空结果回退成 `track`（回退是**确定的**，不是随机）。
/// 形状 = `<index>.<sanitized-track-name>.wav`，`index` 是**工程内轨顺序**的十进制（`1` 起）。
///
/// 序号宽度 = [`STEM_INDEX_MIN_WIDTH`] 起，按 `track_count`（工程里的**全部**轨数，含
/// master —— 宽度只要随总量增长即可，⛔ 不需要精确）加宽。用全量而不是"本批条数"是为了
/// 让"导出其中 2 条"与"导出全部 20 条"给出**同一个文件名**：同一工程的分轨名不随筛选条件
/// 漂移，否则两次导出的产物无法互认。
#[must_use]
pub fn stem_file_name(index: usize, track_count: usize, name: &str) -> String {
    let width = usize::max(STEM_INDEX_MIN_WIDTH, track_count.to_string().len());
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(|ch: char| ch == '.' || ch == '_');
    let base = if trimmed.is_empty() { "track" } else { trimmed };
    format!("{index:0width$}.{base}.{STEM_FILE_EXTENSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清洗规则的唯一权威是 **ADR-0030 D7**：保留 `[0-9A-Za-z._-]`、其余逐个 `_`、
    /// 首尾的 `.`/`_` 剥掉、空结果回退 `track`。下面的期望值全部按这条规则实测得出。
    #[test]
    fn file_name_keeps_only_the_safe_charset_and_pads_the_index() {
        // 空格与两个汉字各换成一个 `_`，尾部的 `__` 属于首尾剥离范围 ⇒ 全消。
        assert_eq!(stem_file_name(1, 3, "Lead Vox 主唱"), "0001.Lead_Vox.wav");
        assert_eq!(stem_file_name(42, 99, "drum-kit.2"), "0042.drum-kit.2.wav");
    }

    #[test]
    fn path_separators_and_dot_only_names_fall_back_to_track() {
        // 纯点号：整个被首尾剥离吃掉 ⇒ 空结果 ⇒ 确定回退。
        assert_eq!(stem_file_name(1, 3, ".."), "0001.track.wav");
        // `/` 换成 `_` 后名字里**不再有分隔符**，中段的 `..` 因此只是普通字符，
        // 既不构成目录穿越也无需改写（`.` 在保留集内）。
        assert_eq!(stem_file_name(1, 3, "a/../../b"), "0001.a_.._.._b.wav");
        assert_eq!(stem_file_name(1, 3, "   "), "0001.track.wav");
    }

    #[test]
    fn index_width_grows_with_the_project_track_count() {
        // 下限 [`STEM_INDEX_MIN_WIDTH`] = 4；总轨数的十进制位数更宽时按位数加宽。
        assert_eq!(stem_file_name(7, 3, "a"), "0007.a.wav");
        assert_eq!(stem_file_name(7, 99_999, "a"), "00007.a.wav");
        assert_eq!(stem_file_name(7, 999_999, "a"), "000007.a.wav");
    }

    /// 两条豁免的 `reason` 必须是**字面量**站点（`crates/yeban-mcp/tests/fault_vocabulary.rs`
    /// 的冻结面只认字面站点），同级键也必须就是黄金表登记的那三个。
    #[test]
    fn skipped_rows_carry_a_literal_reason_and_exactly_three_keys() {
        use std::str::FromStr;
        let track_id = EntityId::from_str("01J8ZR00000000000000000001").expect("合法 ULID");
        for (reason, label) in [
            (SkipReason::MasterBusTrack, "masterBusTrack"),
            (SkipReason::NotASourceNode, "notASourceNode"),
        ] {
            let row = SkippedTrack {
                track_id,
                name: "Lead".to_owned(),
                reason,
            }
            .to_value();
            assert_eq!(row["reason"], json!(label));
            // 键**集合**必须就是黄金表登记的那三个。顺序上 `serde_json::Map` 默认由
            // `BTreeMap` 承载（本仓库不开 `preserve_order`），实测序即字典序 ——
            // 这与 `json!` 里的书写顺序无关，且同样是**确定**的。
            assert_eq!(
                row.as_object().expect("对象").keys().collect::<Vec<_>>(),
                vec!["name", "reason", "trackId"],
                "同级键集合必须和黄金表一致（字典序 = BTreeMap 承载）"
            );
        }
    }
}
