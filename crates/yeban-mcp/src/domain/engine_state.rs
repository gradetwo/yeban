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
//! | `session.undoCursor` | `crate::undo_session::UndoState`（MCP 侧的会话态；`apply` 之后单向同步进 `SessionRuntimeState`） | 两个游标都必须来自**同一**事实（[`super::Domain::session`] 的同步在 `apply` 里做） |
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

use serde_json::{Map, Value};

use yeban_model::{EntityId, SessionRuntimeState, TrackV3, YebanProjectV1};

use super::error::Fault;
use crate::tools::ErrorCode;

/// 宿主注入的**引擎读数镜像**（只读快照，见模块文档的"为什么不是第二份状态"）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineReadings {
    /// 引擎当前**实际运行**的采样率（与工程的采样率对账用）。
    pub sample_rate: u32,
    /// 当前音频回调的缓冲帧数（`yeban-engine` 的设备协商结果）。
    pub buffer_frames: u32,
}

/// 会话读数的 JSON 形态。
///
/// 参数里的 `session` 是**模型层**的会话运行态（唯一权威），`undo_cursor` 是 MCP
/// 侧撤销会话态的读数 —— 两者在 `Domain::apply` 之后必然一致（`Domain::sync_session`）。
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
        "undoCursor": undo_cursor,
    })
}

/// 引擎读数的 JSON 形态（`mirror == None` 时如实报告"不知道"）。
#[must_use]
pub fn engine_value(project: &YebanProjectV1, mirror: Option<EngineReadings>) -> Value {
    let project_rate = project.sample_rate().hz();
    let (buffer_frames, mirror_rate, matches) = match mirror {
        Some(readings) => (
            Value::from(readings.buffer_frames),
            Value::from(readings.sample_rate),
            Value::from(readings.sample_rate == project_rate),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    serde_json::json!({
        "sampleRate": project_rate,
        "sampleRateSource": "project.audio_config.sample_rate",
        "bufferFrames": buffer_frames,
        "bufferSource": if mirror.is_some() { "hostEngineMirror" } else { "unavailable" },
        "mirrorPresent": mirror.is_some(),
        "mirrorSampleRate": mirror_rate,
        "sampleRateMatchesMirror": matches,
    })
}

/// 一条设备链的 JSON 形态（`slotIndex` 就是链上的位置，`ARCH-PDC-001` 的延迟逐台可见）。
#[must_use]
pub fn device_chain_value(track: &TrackV3) -> Value {
    let devices: Vec<Value> = track
        .devices
        .iter()
        .enumerate()
        .map(|(slot_index, device)| {
            serde_json::json!({
                "slotIndex": slot_index,
                "deviceId": device.id.to_canonical_string(),
                "name": device.name.clone(),
                "kind": enum_name(&device.kind),
                "bypassed": device.bypassed,
                // PDC 的唯一来源: 少报/漏报会造成汇合点的相位错位 (ARCH-PDC-001)。
                "latencySamples": device.latency_samples,
                "paramCount": device.params.len(),
                "params": device
                    .params
                    .iter()
                    .enumerate()
                    .map(|(param_index, param)| {
                        serde_json::json!({
                            "paramIndex": param_index,
                            "name": param.name.clone(),
                            "value": param.value,
                            "unit": param.unit.clone(),
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
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
    })
}

/// 组装一次查询的 `data`。
///
/// # Errors
///
/// `track_id` 给定的音轨不存在 → `TRACK_NOT_FOUND`（复用 `YebanProjectV1::track`
/// 的既有语义，不自己查表造一个错误）。
pub fn snapshot(
    project: &YebanProjectV1,
    session: &SessionRuntimeState,
    mirror: Option<EngineReadings>,
    undo_cursor: usize,
    track_id: Option<EntityId>,
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
    data.insert("engine".to_owned(), engine_value(project, mirror));
    data.insert(
        "track".to_owned(),
        track.map_or(Value::Null, device_chain_value),
    );
    data.insert(
        "stateSources".to_owned(),
        serde_json::json!({
            "playhead": "yeban_model::SessionRuntimeState",
            "sampleRate": "project.audio_config.sample_rate",
            "bufferFrames": "host injected EngineReadings mirror",
            "devices": "project.tracks[].devices",
            "undoCursor": "undo_session::UndoState",
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
        let bare = engine_value(&project, None);
        assert_eq!(bare["sampleRate"], 48_000);
        assert_eq!(bare["bufferFrames"], Value::Null);
        assert_eq!(bare["bufferSource"], "unavailable");
        assert_eq!(bare["mirrorPresent"], false);

        let with = engine_value(
            &project,
            Some(EngineReadings {
                sample_rate: 44_100,
                buffer_frames: 128,
            }),
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
            }),
        );
        assert_eq!(matching["sampleRateMatchesMirror"], true);
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
        let value = snapshot(&project, &session, None, 0, None).expect("快照");
        assert_eq!(value["track"], Value::Null);
        assert!(value["stateSources"]["bufferFrames"].is_string());

        let ghost = super::super::ids::deterministic_id("ghost-track");
        let fault = snapshot(&project, &session, None, 0, Some(ghost)).expect_err("音轨不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
        assert_eq!(
            needs_project().domain_code(),
            Some(ErrorCode::NoActiveProject)
        );
    }
}
