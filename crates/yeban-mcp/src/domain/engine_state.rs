//! `yeban_query_engine_state` 的实现 —— 设备链与引擎/会话读数
//! （`ADR-0001` **D46** 的第 2 类能力）。
//!
//! ## 三份状态各有**唯一**来源，本模块一份都不复制
//!
//! `MODEL-ISO-001` 把状态切成三层，本工具的每个字段都锚在**已有的**那一份上：
//!
//! | 字段 | 唯一来源 | 为什么不能另找一处 |
//! | :--- | :--- | :--- |
//! | `engine.sampleRate` | `project.audio_config.sample_rate`（[`YebanProjectV1::sample_rate`]） | 顶层刻意没有 `sample_rate` 冗余副本；设备层注释也点名"采样率唯一事实源是 `audio_config.sample_rate`" |
//! | `session.playheadTicks` / `isPlaying` | [`yeban_model::SessionRuntimeState`]（第 2 层会话运行态，**不落盘**） | 走带位置进持久化文档就会让"同一工程 ⇒ 同一文档"失效 |
//! | `session.undoCursor` | `crate::undo_session::UndoState::display` 的 `undoable`（= 从根算起**已应用**的 op 数；⭐ `#177` 与 `yeban_undo` 响应的 `before.undoable` **同一个**数） | 两个读数必须同源：`undone`（已撤销步数）曾是这里的旧口径，与 `undo` 响应差一个"还能撤几步" ⇒ 同一会话读出 `0` / `1` 两个数（坑⑪）。⚠ `SessionRuntimeState::undo_cursor`（模型层会话态的 `UndoCursor`）仍是"已撤销步数"，它**不是**本字段的来源 |
//! | `track.devices[]` | `TrackV3::devices`（工程文档） | `DeviceDefinition::latency_samples` 是 PDC 的唯一来源（`ARCH-PDC-001`） |
//! | `engine.bufferFrames` | 宿主**注入**的 [`EngineReadings`] 镜像 | 缓冲帧数住在 `yeban-engine` 的设备协商结果里；本 crate **不依赖** `yeban-engine`（零新增依赖），因此只能读宿主交进来的**只读快照** |
//!
//! ## 为什么"镜像"不是"第二份状态"
//!
//! [`EngineReadings`] 满足三条：
//!
//! 1. **只读**：它是 `Copy` 的值，本 crate 没有任何工具能写它；注入口
//!    [`super::Domain::set_engine_readings`] 只对宿主（形态 A 的 `yeban-app`）与判据开放；
//! 2. **不含会话态**：缓冲帧数是**设备**事实，走带位置/播放状态**不在**镜子里
//!    （它们从 `SessionRuntimeState` 读）—— 因此不存在"同一件事有两个来源"；
//! 3. **自报家门**：它同时上报自己看到的采样率，响应里给出
//!    `sampleRateMatchesMirror` —— 镜像与工程**不一致时看得见**，而不是悄悄二选一。
//!
//! 没有宿主注入时（形态 B 的 stdio 二进制、判据）缓冲读数是 `null` 且
//! `bufferSource = "unavailable"`：**如实说明不知道**，绝不编一个 128/256。
//!
//! ## 响度读数怎么**越过控制面**到客户端（`docs/ledger/open-questions.md` 问题 2 选 (a)）
//!
//! 契约侧的五个字段（`integratedLufs` / `momentaryLufs` / `shortTermLufs` /
//! `loudnessRangeLu` / `truePeakDbfs`）由宿主注入，与传输无关。这里补的是**交付**：
//! 一次注入 = 一次**修订**（单调递增的 `readingsRevision`），客户端拿上一次的修订当游标
//! （`since`），就能只取到**还没有见过**的那几条读数，而不是反复拉同一份快照。
//!
//! ### 为什么是"游标 + 有界尾部"而不是服务端推送（实测的传输层事实）
//!
//! `crate::transport::http` 的形态是**一个连接一个请求、一个响应**：没有 keep-alive、
//! 没有 `Transfer-Encoding: chunked`、没有 HTTP/2，响应写完就 `Connection: close`
//! （见那个文件头部的"边界（明确没做）"）。因此"服务端在同一条连接上主动写一条
//! notification"**无法**在不发明第二套机制的前提下表达 —— 而账本问题 2 的 (c) 已经预先
//! 拒绝过并行机制（第二套鉴权 / 第二次挂载正是 `MUST-GATE-009` 要防的漂移）。
//!
//! 因此选了 (a) 的最小诚实形态：**同一个方法、同一个 token、同一个 socket**，
//! 多一个可选的 `since` 游标。它不是推送，但它让"客户端收到更新"变成**有修订号可核对**
//! 的事实（`readingsRevision` 单调，`updates[*].revision` 严格递增），而不是"再读一遍
//! 看有没有变化"。
//!
//! ### 不丢更新的口径（诚实边界）
//!
//! 尾部只保留最近 [`READINGS_TAIL_CAPACITY`] 条读数。客户端游标**比保留窗口更旧**时，
//! `readingsStream.agedOut = true` 且 `updates` 为空 —— 这是**如实报告"你错过了"**，
//! 不是静默地少给几条（ADR-0001 D23 的同一条纪律：看不见的丢失比丢失更糟）。
//! 客户端看到它就重新同步一次（缺省调用即为当前完整读数）。

use serde_json::{Map, Value};

use yeban_model::{
    DeviceKind, EntityId, RoutingKind, SessionRuntimeState, TrackV3, YebanProjectV1,
};

use super::error::Fault;
use crate::effect_params;
use crate::tools::ErrorCode;

/// 会话里保留多少条**最近**的引擎读数（游标窗口）。
///
/// 与 `yeban-engine` 的 `DEFAULT_METER_CAPACITY` 同量级：宿主按控制面节奏注入（不是按
/// 音频量子），64 条足以覆盖一个客户端两次调用之间的突发；超出即"太旧"并被如实上报。
pub const READINGS_TAIL_CAPACITY: usize = 64;

/// 宿主注入的**引擎读数镜像**（只读快照，见模块文档的"为什么不是第二份状态"）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineReadings {
    /// 引擎当前**实际运行**的采样率（与工程的采样率对账用）。
    pub sample_rate: u32,
    /// 当前音频回调的缓冲帧数（`yeban-engine` 的设备协商结果）。
    pub buffer_frames: u32,
    /// `[ARCH-UI-002]` 主母线**积分**响度（LUFS）。`None` = **尚未测量**（独立 stdio 服务器今天即此 ⇒ 如实报 `null`）。
    pub integrated_lufs: Option<f32>,
    /// 瞬时窗口 LUFS。
    pub momentary_lufs: Option<f32>,
    /// 短时窗口 LUFS。
    pub short_term_lufs: Option<f32>,
    /// 响度范围（LRA，LU）。
    pub loudness_range_lu: Option<f32>,
    /// 真峰值（dBFS）。
    pub true_peak_dbfs: Option<f32>,
    /// ⭐ `#290`：引擎累计**真的生效**过的量化场景启动次数
    /// （`EngineStats::scene_launches_applied` 的只读镜像）。
    ///
    /// `None` = 这一代引擎拿不到该读数（设备腿的运行时在 cpal 回调线程上，
    /// `EngineHost` 没有它的 stats）⇒ 响应里 `sceneLaunchesApplied` 如实为 `null`，
    /// ⛔ 不沿用上一代引擎的旧数字。
    pub scene_launches_applied: Option<u64>,
}

impl Default for EngineReadings {
    /// 全部读数**缺席**（与"测得静音"区分开 —— 账本第 327/328 轮）。
    fn default() -> Self {
        Self {
            sample_rate: 0,
            buffer_frames: 0,
            integrated_lufs: None,
            momentary_lufs: None,
            short_term_lufs: None,
            loudness_range_lu: None,
            true_peak_dbfs: None,
            scene_launches_applied: None,
        }
    }
}

/// 会话读数的 JSON 形态。
///
/// 参数里的 `session` 是**模型层**的会话运行态（唯一权威）。
///
/// ⭐ `#177` 坑⑪：`undo_cursor` 现在是 **`UndoState::display().undoable`**
/// （= 还能撤销的步数 = 从根算起已应用的 op 数），**不是**旧口径
/// `UndoState::undone()`（已撤销步数）。它与 `yeban_undo` 响应的
/// `before.undoable` 是**同一个**数（两处都走 `Domain::undo_display`）。
/// ⚠ 模型层 `SessionRuntimeState::undo_cursor` 仍是 `UndoCursor`（已撤销步数）——
/// 本字段**不**从它读，两者语义不同（这是"改名不改口径"之外的诚实边界：
/// 工具面的 `undoCursor` 回答"现在处在历史第几步"，模型字段回答"撤了几步"）。
#[must_use]
pub fn session_value(
    session: &SessionRuntimeState,
    undo_cursor: usize,
    numerator: u8,
    denominator: u8,
) -> Value {
    let bar = session.playhead_bar_and_offset(numerator, denominator);
    serde_json::json!({
        "playheadTicks": session.playhead_ticks(),
        "isPlaying": session.is_playing(),
        // 模型 `TransportState::Recording` 是**预留**档（模型里没有任何入口能到达它），
        // 因此这里只有两态：不假装支持录音。
        "transportState": if session.is_playing() { "playing" } else { "stopped" },
        "ticksPerBeat": SessionRuntimeState::ticks_per_beat(),
        "playheadBar": bar.map(|(bar, _)| bar),
        "playheadBarOffsetTicks": bar.map(|(_, offset)| offset),
        "taskCount": session.tasks.len(),
        "pluginPids": session.plugin_pids().into_iter().collect::<Vec<_>>(),
        "openWindowCount": session.open_windows.len(),
        // `#177`：口径 = 可撤销步数（= 已应用 op 数），与 `yeban_undo` 的
        // `before.undoable` 逐值相同 —— 由 `Domain::plan_query_engine_state`
        // 用同一个 `undo_display().undoable` 传入。
        "undoCursor": undo_cursor,
    })
}

/// 引擎读数的 JSON 形态（`mirror == None` 时如实报告"不知道"）。
#[must_use]
pub fn engine_value(
    project: &YebanProjectV1,
    mirror: Option<EngineReadings>,
    scene_launches_applied: Option<u64>,
) -> Value {
    let project_rate = project.sample_rate().hz();
    let (buffer_frames, mirror_rate, matches) = match mirror {
        Some(readings) => (
            Value::from(readings.buffer_frames),
            Value::from(readings.sample_rate),
            Value::from(readings.sample_rate == project_rate),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    // ⭐ `#290`：applied 计数优先取宿主**单独**注入的那一格（心跳每跳更新），
    // 其次取完整读数镜像里的同名字段（老客户端 / 判据只注入 `EngineReadings` 时）。
    let applied = scene_launches_applied
        .or_else(|| mirror.and_then(|readings| readings.scene_launches_applied));
    serde_json::json!({
        "sampleRate": project_rate,
        "sampleRateSource": "project.audio_config.sample_rate",
        "bufferFrames": buffer_frames,
        "bufferSource": if mirror.is_some() { "hostEngineMirror" } else { "unavailable" },
        "mirrorPresent": mirror.is_some(),
        // `[ARCH-UI-002]` 响度读数：**宿主注入**，未测量即 `null`（不编造 0 —— 账本第 327/328 轮）。
        "integratedLufs": mirror.and_then(|readings| readings.integrated_lufs),
        "momentaryLufs": mirror.and_then(|readings| readings.momentary_lufs),
        "shortTermLufs": mirror.and_then(|readings| readings.short_term_lufs),
        "loudnessRangeLu": mirror.and_then(|readings| readings.loudness_range_lu),
        "truePeakDbfs": mirror.and_then(|readings| readings.true_peak_dbfs),
        // ⭐ `#290`：量化场景启动的累计生效次数（宿主心跳注入；`None` ⇒ `null`）。
        "sceneLaunchesApplied": applied,
        "sceneLaunchesAppliedSource": if applied.is_some() { "hostEngineMirror" } else { "unavailable" },
        "mirrorSampleRate": mirror_rate,
        "sampleRateMatchesMirror": matches,
    })
}

/// 一条读数的 JSON 记录（游标段与当前读数**共用同一份字段拼装**，不许漂移）。
///
/// `revision` 是这条读数被注入时的修订号（严格递增）。
#[must_use]
pub fn readings_record(revision: u64, readings: EngineReadings) -> Value {
    serde_json::json!({
        "revision": revision,
        "sampleRate": readings.sample_rate,
        "bufferFrames": readings.buffer_frames,
        // 五个响度字段与当前读数**同一个拼法**（`[ARCH-UI-002]`；未测量 = `null`）。
        "integratedLufs": readings.integrated_lufs,
        "momentaryLufs": readings.momentary_lufs,
        "shortTermLufs": readings.short_term_lufs,
        "loudnessRangeLu": readings.loudness_range_lu,
        "truePeakDbfs": readings.true_peak_dbfs,
        // ⭐ `#290`：与当前读数同一口径；没有注入时为 `null`。
        "sceneLaunchesApplied": readings.scene_launches_applied,
    })
}

/// 游标段的 JSON 形态：`since` 之后**还没有被这个客户端见过**的读数（修订号严格递增）。
///
/// - `since` 比窗口更旧（`since + 1 < oldest`）⇒ `agedOut: true`、`updates: []`：
///   如实说"你错过了"，而不是假装给全了；
/// - `since` 超前（客户端拿了一个更大的修订号）⇒ 恒为 `agedOut: false` + 空更新：
///   服务端不为非法的游标编造读数（也不报错 —— 客户端可能只是重放过）。
///
/// `revision` 是会话**当前**的修订号（`engine.readingsRevision` 的同一个数）。
#[must_use]
pub fn readings_stream_value(
    since: Option<u64>,
    revision: u64,
    tail: &[(u64, EngineReadings)],
) -> Value {
    let Some(since) = since else {
        // 缺省调用：没有游标 ⇒ 没有增量段（只有 `engine.readingsRevision` 那一份当前读数）。
        return Value::Null;
    };
    let oldest = tail.first().map(|(revision, _)| *revision);
    let aged_out = oldest.is_some_and(|oldest| since.saturating_add(1) < oldest);
    let updates: Vec<Value> = if aged_out {
        Vec::new()
    } else {
        tail.iter()
            .filter(|(revision, _)| *revision > since)
            .map(|(revision, readings)| readings_record(*revision, *readings))
            .collect()
    };
    serde_json::json!({
        "since": since,
        "revision": revision,
        "buffered": tail.len(),
        "agedOut": aged_out,
        "updates": updates,
    })
}

/// 一条设备链的 JSON 形态（`slotIndex` 就是链上的位置，`ARCH-PDC-001` 的延迟逐台可见）。
///
/// ⭐ #156：每个参数多报三个只读键 —— `recognised` / `canonicalName` / `effect`，
/// 并在轨对象上多一段 `inserts[]`（设备名 + 槽位 + 已识别/未识别的参数名）。
/// 这是让 Agent **确认自己写对了**的验收面：写了 `reverb_mix` 会出现在
/// `unrecognisedParams` 里，而不会被误认为生效。既有字段一个都不删。
#[must_use]
pub fn device_chain_value(track: &TrackV3) -> Value {
    let mut devices: Vec<Value> = Vec::with_capacity(track.devices.len());
    let mut inserts: Vec<Value> = Vec::with_capacity(track.devices.len());
    for (slot_index, device) in track.devices.iter().enumerate() {
        let mut effects: Vec<&'static str> = Vec::new();
        let mut recognised_params: Vec<&'static str> = Vec::new();
        let mut unrecognised_params: Vec<String> = Vec::new();
        for param in &device.params {
            match effect_params::recognise(&param.name) {
                Some((group, canonical)) => {
                    recognised_params.push(canonical.name);
                    if !effects.contains(&group.id) {
                        effects.push(group.id);
                    }
                }
                None => unrecognised_params.push(param.name.clone()),
            }
        }
        // 引擎只从**非旁通的内置效果器**上取参数（`insert.rs` 的规则 1+2）。
        let active = !device.bypassed && device.kind == DeviceKind::InternalEffect;
        let params: Vec<Value> = device
            .params
            .iter()
            .enumerate()
            .map(|(param_index, param)| {
                let recognised = effect_params::recognise(&param.name);
                serde_json::json!({
                    "paramIndex": param_index,
                    "name": param.name.clone(),
                    "value": param.value,
                    "unit": param.unit.clone(),
                    // ⭐ 只读验收读数（既有四个键不动）。
                    "recognised": recognised.is_some(),
                    "canonicalName": recognised.map(|(_, canonical)| canonical.name),
                    "effect": recognised.map(|(group, _)| group.id),
                })
            })
            .collect();
        devices.push(serde_json::json!({
            "slotIndex": slot_index,
            "deviceId": device.id.to_canonical_string(),
            "name": device.name.clone(),
            "kind": enum_name(&device.kind),
            "bypassed": device.bypassed,
            // PDC 的唯一来源: 少报/漏报会造成汇合点的相位错位 (ARCH-PDC-001)。
            "latencySamples": device.latency_samples,
            "paramCount": device.params.len(),
            "params": params,
            // ⭐ #156：这台设备是否真的会武装引擎内置效果器。
            "active": active,
            "effects": effects,
            "recognisedParamCount": recognised_params.len(),
            "unrecognisedParamCount": unrecognised_params.len(),
        }));
        inserts.push(serde_json::json!({
            "slotIndex": slot_index,
            "deviceId": device.id.to_canonical_string(),
            "deviceName": device.name.clone(),
            "deviceKind": enum_name(&device.kind),
            "bypassed": device.bypassed,
            "active": active,
            "effects": effects,
            "recognisedParams": recognised_params,
            "unrecognisedParams": unrecognised_params,
        }));
    }
    // 链的总延迟 = 逐台求和（u64 饱和: 病态文档不会把读数掀翻成 panic）。
    let total_latency: u64 = track
        .devices
        .iter()
        .map(|device| u64::from(device.latency_samples))
        .fold(0_u64, u64::saturating_add);
    serde_json::json!({
        "id": track.id.to_canonical_string(),
        "name": track.name.clone(),
        "kind": enum_name(&track.kind),
        "deviceCount": track.devices.len(),
        "totalLatencySamples": total_latency,
        "devices": devices,
        // ⭐ #156：每轨 insert 摘要（设备名 + 槽位 + 已识别/未识别的参数名）。
        "inserts": inserts,
    })
}

/// 一条路由边是不是**发送**（`SendToAux`）—— 只看种类，不看增益。
///
/// `source` 给定时只报这条音轨发出的边；缺省 = 报工程里的全部发送边。
/// 增益是 `Option<f32>` 的**字面**形态：`null` = 单位增益（与 `0.0` 可区分）。
#[must_use]
pub fn send_edges_value(project: &YebanProjectV1, source: Option<EntityId>) -> Value {
    let name_of = |id: EntityId| {
        project
            .track(&id)
            .ok()
            .map(|track| track.name.clone())
            .map_or(Value::Null, Value::from)
    };
    let sends: Vec<Value> = project
        .routing_graph
        .edges
        .values()
        .filter(|edge| edge.kind == RoutingKind::SendToAux)
        .filter(|edge| source.is_none_or(|id| edge.source_node == id))
        .map(|edge| {
            serde_json::json!({
                "edgeId": edge.id.to_canonical_string(),
                "kind": enum_name(&edge.kind),
                "sourceNode": edge.source_node.to_canonical_string(),
                "sourceName": name_of(edge.source_node),
                "destinationNode": edge.destination_node.to_canonical_string(),
                "destinationName": name_of(edge.destination_node),
                "gainDb": edge.gain_db,
                "gainSource": "project.routing_graph.edges[].gain_db (null = 单位增益)",
            })
        })
        .collect();
    Value::Array(sends)
}

/// 读数**游标**的三样东西：当前修订号、保留窗口、以及客户端的游标。
///
/// 收成一个结构体而不是三个参数：`clippy::too_many_arguments` 在本仓是承重信号
/// （见 `Domain::SessionSeed` 的同款理由），而且这三者**必须一起**给出 ——
/// 分开传就允许"修订号与窗口不是同一次读取"这种自相矛盾的调用。
#[derive(Clone, Copy, Debug)]
pub struct ReadingsCursor<'a> {
    /// 会话当前的读数修订号（`0` = 从未注入）。
    pub revision: u64,
    /// 最近注入的读数（含修订号），按修订递增。
    pub tail: &'a [(u64, EngineReadings)],
    /// 客户端的游标（`None` = 本次调用不带增量段）。
    pub since: Option<u64>,
}

/// 组装一次查询的 `data`。
///
/// `readings` 是客户端的**读数游标**（上一个它见过的 `readingsRevision`）；缺省 ⇒
/// 响应里没有 `readingsStream` 段（只有当前读数）。
///
/// # Errors
///
/// `track_id` 给定的音轨不存在 → `TRACK_NOT_FOUND`（复用 `YebanProjectV1::track`
/// 的既有语义，不自己查表造一个错误）。
pub fn snapshot(
    project: &YebanProjectV1,
    session: &SessionRuntimeState,
    mirror: Option<EngineReadings>,
    scene_launches_applied: Option<u64>,
    undo_cursor: usize,
    track_id: Option<EntityId>,
    readings: ReadingsCursor<'_>,
) -> Result<Value, Fault> {
    let track = track_id
        .map(|id| {
            project
                .track(&id)
                .map_err(|error| super::error::from_model("音轨查找", &error))
        })
        .transpose()?;
    let mut data = Map::new();
    data.insert(
        "session".to_owned(),
        session_value(
            session,
            undo_cursor,
            project.time_signature.numerator,
            project.time_signature.denominator,
        ),
    );
    let mut engine = engine_value(project, mirror, scene_launches_applied);
    if let Value::Object(fields) = &mut engine {
        // 客户端从这里取下一次调用要用的游标（`readingsRevision`）。
        fields.insert(
            "readingsRevision".to_owned(),
            Value::from(readings.revision),
        );
    }
    data.insert("engine".to_owned(), engine);
    let stream = readings_stream_value(readings.since, readings.revision, readings.tail);
    if !stream.is_null() {
        data.insert("readingsStream".to_owned(), stream);
    }
    data.insert(
        "track".to_owned(),
        track.map_or(Value::Null, |track| {
            let mut value = device_chain_value(track);
            // ⭐ #156：这条音轨**发出**的发送边（目的地 + 增益 + edgeId）。
            if let Value::Object(fields) = &mut value {
                fields.insert(
                    "sends".to_owned(),
                    send_edges_value(project, Some(track.id)),
                );
            }
            value
        }),
    );
    // ⭐ #156：工程里的**全部**发送边（edgeId / gainDb / 目的地）。
    //
    // ⚠ **只在点名了一条音轨时**给出：发送边带着音轨身份与显示名（工程内容），
    // 而缺省调用（`trackId` 不带）同时是 `yeban_export_diagnostics` 喂
    // `engine-state.json` 的那条路径 —— 那个包承诺 `projectIncluded:false`，
    // 它的判据（`diagnostics_bundle_content_matches_the_project_included_flag`）
    // 逐字节扫描工程标记。缺省把工程标记写进诊断包就是**隐私读数说谎**。
    if track_id.is_some() {
        data.insert("sends".to_owned(), send_edges_value(project, None));
    }
    data.insert(
        "stateSources".to_owned(),
        serde_json::json!({
            "playhead": "yeban_model::SessionRuntimeState",
            "sampleRate": "project.audio_config.sample_rate",
            "bufferFrames": "host injected EngineReadings mirror",
            "devices": "project.tracks[].devices",
            "inserts": "project.tracks[].devices (recognised against crate::effect_params)",
            "sends": "project.routing_graph.edges where kind == SendToAux",
            "undoCursor": "undo_session::UndoState::display().undoable (applied op count; same value as yeban_undo before.undoable) [#177]",
            "readingsRevision": "host injected EngineReadings mirror (monotonic)",
            "sceneLaunchesApplied": "host injected EngineStats::scene_launches_applied (None = this engine generation cannot read it)",
        }),
    );
    Ok(Value::Object(data))
}

/// `serde` 派生名（与 `project.json` 同一份词汇表；失败时 `"unknown"`，绝不 panic）。
fn enum_name<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// 当前没有活跃工程时的**明确**错误（本工具需要工程才能报采样率与设备链）。
#[must_use]
pub fn needs_project() -> Fault {
    Fault::domain(ErrorCode::NoActiveProject, "当前没有活跃工程")
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    fn lead(project: &YebanProjectV1) -> EntityId {
        project
            .tracks
            .values()
            .find(|track| !track.devices.is_empty())
            .expect("样本里必须有带设备的音轨")
            .id
    }

    #[test]
    fn device_chain_reports_slots_latency_and_params() {
        let project = filled_project();
        let track = project.track(&lead(&project)).expect("音轨");
        let value = device_chain_value(track);
        assert_eq!(value["deviceCount"], 1);
        assert_eq!(value["devices"][0]["slotIndex"], 0);
        assert_eq!(value["devices"][0]["latencySamples"], 32);
        assert_eq!(value["devices"][0]["paramCount"], 2);
        assert_eq!(value["devices"][0]["params"][0]["name"], "cutoff");
        assert_eq!(value["devices"][0]["params"][0]["unit"], "Hz");
        assert_eq!(value["totalLatencySamples"], 32);
        assert_eq!(value["kind"], "Midi");
        assert_eq!(value["devices"][0]["kind"], "InternalInstrument");
    }

    #[test]
    fn engine_value_reads_the_sample_rate_from_the_project_and_the_buffer_from_the_mirror() {
        let project = filled_project();
        let bare = engine_value(&project, None, None);
        assert_eq!(bare["sampleRate"], 48_000);
        assert_eq!(bare["bufferFrames"], Value::Null);
        assert_eq!(bare["bufferSource"], "unavailable");
        assert_eq!(bare["mirrorPresent"], false);

        let with = engine_value(
            &project,
            Some(EngineReadings {
                sample_rate: 44_100,
                buffer_frames: 128,
                ..EngineReadings::default()
            }),
            None,
        );
        assert_eq!(with["bufferFrames"], 128);
        assert_eq!(with["bufferSource"], "hostEngineMirror");
        assert_eq!(
            with["sampleRateMatchesMirror"], false,
            "镜像与工程不一致必须看得见"
        );
        let matching = engine_value(
            &project,
            Some(EngineReadings {
                sample_rate: 48_000,
                buffer_frames: 256,
                ..EngineReadings::default()
            }),
            None,
        );
        assert_eq!(matching["sampleRateMatchesMirror"], true);
    }

    #[test]
    fn scene_launches_applied_is_null_without_injection_and_echoed_when_injected() {
        let project = filled_project();
        let absent = engine_value(&project, None, None);
        assert!(absent["sceneLaunchesApplied"].is_null());
        assert_eq!(absent["sceneLaunchesAppliedSource"], "unavailable");
        let injected = engine_value(&project, None, Some(3));
        assert_eq!(injected["sceneLaunchesApplied"], 3);
        assert_eq!(injected["sceneLaunchesAppliedSource"], "hostEngineMirror");
    }

    #[test]
    fn session_value_reports_transport_from_the_model_state() {
        let mut session = SessionRuntimeState::new();
        let stopped = session_value(&session, 0, 4, 4);
        assert_eq!(stopped["isPlaying"], false);
        assert_eq!(stopped["transportState"], "stopped");
        assert_eq!(stopped["ticksPerBeat"], 960);
        session.seek_ticks(3840);
        session.play();
        let playing = session_value(&session, 2, 4, 4);
        assert_eq!(playing["playheadTicks"], 3840);
        assert_eq!(playing["transportState"], "playing");
        assert_eq!(playing["playheadBar"], 1);
        assert_eq!(playing["playheadBarOffsetTicks"], 0);
        assert_eq!(playing["undoCursor"], 2);
    }

    #[test]
    fn snapshot_is_null_track_by_default_and_track_not_found_when_asked() {
        let project = filled_project();
        let session = SessionRuntimeState::new();
        let value = snapshot(
            &project,
            &session,
            None,
            None,
            0,
            None,
            ReadingsCursor {
                revision: 0,
                tail: &[],
                since: None,
            },
        )
        .expect("快照");
        assert_eq!(value["track"], Value::Null);
        assert!(value["stateSources"]["bufferFrames"].is_string());
        // 没有游标 ⇒ 没有增量段（缺省调用的形状不变）。
        assert!(value.get("readingsStream").is_none());
        assert_eq!(value["engine"]["readingsRevision"], 0);

        let ghost = super::super::ids::deterministic_id("ghost-track");
        let fault = snapshot(
            &project,
            &session,
            None,
            None,
            0,
            Some(ghost),
            ReadingsCursor {
                revision: 0,
                tail: &[],
                since: None,
            },
        )
        .expect_err("音轨不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
        assert_eq!(
            needs_project().domain_code(),
            Some(ErrorCode::NoActiveProject)
        );
    }

    /// 游标段的**四条**口径：只给没见过的、严格递增、太旧就如实说 `agedOut`、
    /// 且五个响度字段与当前读数是**同一个拼法**（未测量 = `null`）。
    #[test]
    fn the_readings_cursor_only_hands_out_what_the_client_has_not_seen() {
        let measured = EngineReadings {
            sample_rate: 48_000,
            buffer_frames: 128,
            integrated_lufs: Some(-14.0),
            momentary_lufs: Some(-13.5),
            short_term_lufs: Some(-13.8),
            loudness_range_lu: Some(6.0),
            true_peak_dbfs: Some(-1.0),
            scene_launches_applied: Some(2),
        };
        let tail = [(3_u64, measured), (4, EngineReadings::default())];

        // ---- 客户端说"我见过 3" ⇒ 只拿到 4（严格大于游标） ----
        let stream = readings_stream_value(Some(3), 4, &tail);
        assert_eq!(stream["since"], 3);
        assert_eq!(stream["revision"], 4);
        assert_eq!(stream["buffered"], 2);
        assert_eq!(stream["agedOut"], false);
        let updates = stream["updates"].as_array().expect("updates 数组");
        assert_eq!(updates.len(), 1, "只给游标之后的: {stream}");
        assert_eq!(updates[0]["revision"], 4);
        // 未测量的那一条：五个字段**全部** `null`（不是 0）—— 与契约同一条纪律。
        for key in [
            "integratedLufs",
            "momentaryLufs",
            "shortTermLufs",
            "loudnessRangeLu",
            "truePeakDbfs",
        ] {
            assert!(
                updates[0][key].is_null(),
                "未测量的 `{key}` 必须是 null: {stream}"
            );
        }

        // ---- 客户端落后于窗口 ⇒ 如实说"你错过了", 不静默少给 ----
        let aged = readings_stream_value(Some(1), 4, &tail);
        assert_eq!(aged["agedOut"], true);
        assert_eq!(aged["updates"].as_array().expect("数组").len(), 0);

        // ---- 客户端已经是最新（或超前）⇒ 空更新, 但**不**报 agedOut ----
        let current = readings_stream_value(Some(4), 4, &tail);
        assert_eq!(current["agedOut"], false);
        assert_eq!(current["updates"].as_array().expect("数组").len(), 0);
        let ahead = readings_stream_value(Some(99), 4, &tail);
        assert_eq!(ahead["agedOut"], false);
        assert_eq!(ahead["updates"].as_array().expect("数组").len(), 0);

        // ---- 已测量的那条：五个字段**在容差内**回显（与 `engine_value` 同一份拼装） ----
        let full = readings_stream_value(Some(2), 3, &tail);
        let record = &full["updates"][0];
        for (key, want) in [
            ("integratedLufs", -14.0_f64),
            ("momentaryLufs", -13.5),
            ("shortTermLufs", -13.8),
            ("loudnessRangeLu", 6.0),
            ("truePeakDbfs", -1.0),
        ] {
            let got = record[key].as_f64().expect("读数应是数值");
            assert!((got - want).abs() < 1e-4, "{key}: got {got}, want {want}");
        }

        // ---- 无游标 ⇒ `null` 段（缺省调用不带增量段） ----
        assert!(readings_stream_value(None, 4, &tail).is_null());
        // 空窗口 + 非零游标：没有东西可给，但**也不**是 agedOut（没有任何东西被丢）。
        assert_eq!(readings_stream_value(Some(0), 0, &[])["agedOut"], false);
    }

    #[test]
    fn loudness_keys_are_null_when_unset_and_echoed_when_set() {
        let project = filled_project();
        let keys = [
            "integratedLufs",
            "momentaryLufs",
            "shortTermLufs",
            "loudnessRangeLu",
            "truePeakDbfs",
        ];
        // 无镜像：**不知道**，如实 null（不编造 0）。
        let absent = engine_value(&project, None, None);
        for key in keys {
            assert!(absent[key].is_null(), "{key} 无镜像时必须是 null");
        }
        // 有镜像但**尚未测量**：仍为 null —— 与"测得静音"区分（账本第 327/328 轮）。
        let unmeasured = engine_value(&project, Some(EngineReadings::default()), None);
        for key in keys {
            assert!(unmeasured[key].is_null(), "{key} 未测量时必须是 null");
        }
        // 已测量：在**容差内**回显（读数是 `f32`，经载荷成 `f64` 后不保证逐位相等 —— 账本第 330 轮）。
        let measured = engine_value(
            &project,
            Some(EngineReadings {
                integrated_lufs: Some(-14.0),
                momentary_lufs: Some(-13.5),
                short_term_lufs: Some(-13.8),
                loudness_range_lu: Some(6.0),
                true_peak_dbfs: Some(-1.0),
                ..EngineReadings::default()
            }),
            None,
        );
        for (key, want) in [
            ("integratedLufs", -14.0_f64),
            ("momentaryLufs", -13.5),
            ("shortTermLufs", -13.8),
            ("loudnessRangeLu", 6.0),
            ("truePeakDbfs", -1.0),
        ] {
            let got = measured[key].as_f64().expect("读数应是数值");
            assert!(
                (got - want).abs() < 1e-4,
                "{key} 应在容差内回显：got {got}, want {want}"
            );
        }
    }
}
