//! `yeban_launch_scene` 的领域实现 —— 把一次**场景启动**交给宿主（`#290`）。
//!
//! ## 为什么它**不**碰引擎
//!
//! `yeban-engine` 在 `scripts/gates/check_mcp_dependency_direction.py` 的 `FORBIDDEN`
//! 里（`yeban-engine # 设备/回调`）：本 crate 是"轻量 crate，不拖音频栈进 MCP"。
//! 因此本模块**只**做三件纯事：
//!
//! 1. 从活跃工程解析 `sceneId`（身份 / 下标 / 名称 / tempo 覆盖 / 量化档 / 拍号）；
//! 2. 检查宿主是否接上了**引擎口**（[`super::Domain::attach_scene_launch_host`]）；
//! 3. 把一条 [`SceneLaunchRequest`] 放进会话内**有界**队列，等宿主下一次心跳取走。
//!
//! 真正的 `EngineRuntime::queue_scene_launch` 由形态 A 的 `yeban-app` 在**它自己的**
//! 线程上调用（`crates/yeban-app/src/mcp_mount.rs` 的 `apply_scene_launch_requests`）。
//! 这条分工是**结构**上的：本 crate 的依赖图里根本没有引擎，⛔ 不是"约定"。
//!
//! ## 形态 B 为什么必须响亮报错
//!
//! 独立 `yeban-mcp` 二进制（`src/bin/yeban-mcp.rs`）不托管引擎，也没有宿主 attach 口
//! ⇒ 没有宿主时本模块返回 `CONFLICT`（`reason: "engineUnavailable"`），
//! **不**返回 `status:"success"` + `engine:null`：后者是"静默成功"，与 `#324` 相悖。
//!
//! ## 量化档不是本模块算的
//!
//! `Off` / `Bar` / `TwoBars` / `FourBars` 的**生效帧**由引擎在控制路径用
//! `next_bar_tick` + `frames_for_tick` 算（`crates/yeban-engine/src/rt.rs` 的
//! `scene_launch_switch_frame`）。本模块只把工程配置的量化档原样交出去 ——
//! ⛔ 不在这里另算一套，否则"何时生效"会出现第二份口径。

use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::project::{MAX_BPM, MIN_BPM};
use yeban_model::{EntityId, LaunchQuantization, TimeSignature, YebanProjectV1};

use super::error::Fault;
use crate::tools::ErrorCode;

/// 会话内**最多**积压多少条尚未被宿主取走的场景启动请求。
///
/// 有界是纪律：宿主线程若长时间不心跳（被阻塞 / 事件循环没起），无界队列会把
/// 控制面变成一条内存泄漏。64 与引擎事件环容量（`EVENT_CAPACITY = 256`）同量级，
/// 且远大于"一次心跳间隔内人类能按几次"。
pub const MAX_PENDING_SCENE_LAUNCHES: usize = 64;

/// 一次**已经解析过**的场景启动请求（本 crate 的引擎无关形态）。
///
/// 字段与 `yeban_engine::rt::EngineRuntime::queue_scene_launch` 的入参**一一对应**
/// （⛔ 不在这里另算生效帧 —— 那是引擎的事）。形态 A 的宿主把它逐字段转成
/// `crate::engine_host::SceneLaunchRequest`，转换是**纯复制**，不读第二份工程状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLaunchRequest {
    /// 目标场景身份（引擎只把它当读数；解析失败不阻止启动 —— 但本工具要求它**必须**在工程里）。
    pub scene_id: EntityId,
    /// 该场景的 tempo 覆盖（`SceneV3::tempo`）或工程速度（`None` 时）。
    pub tempo_bpm: f64,
    /// 工程配置的量化档（`transport.launch_quantization`）。
    pub quantization: LaunchQuantization,
    /// 工程拍号（引擎快照里没有 ⇒ 由控制面给）。
    pub time_signature: TimeSignature,
}

/// 解析后的场景读数（响应与 `dryRun` 预览**共用同一份**拼装）。
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedScene {
    /// 交给宿主的请求。
    pub request: SceneLaunchRequest,
    /// 场景在 `project.scenes` 里的**投影下标**（`BTreeMap` 迭代顺序；只作读数）。
    pub scene_index: usize,
    /// 场景显示名。
    pub scene_name: String,
    /// tempo 的来源：`"sceneOverride"`（场景自己的覆盖）或 `"project"`（跟随工程）。
    pub tempo_source: &'static str,
}

impl ResolvedScene {
    /// 成功响应 / `dryRun` 预览的 `data`。
    ///
    /// `queued` 在 `plan` 相位是 `false`（只读预览），在 `apply` 相位改成 `true`。
    /// `applied` **恒为 `false`**：本响应只代表"已交给宿主"，⛔ 不代表引擎已经生效。
    #[must_use]
    pub fn data(&self, queued: bool) -> Value {
        let mut data = Map::new();
        data.insert("queued".to_owned(), Value::from(queued));
        data.insert("applied".to_owned(), Value::from(false));
        data.insert(
            "sceneId".to_owned(),
            Value::from(self.request.scene_id.to_canonical_string()),
        );
        data.insert("sceneIndex".to_owned(), Value::from(self.scene_index));
        data.insert("sceneName".to_owned(), Value::from(self.scene_name.clone()));
        data.insert("tempoBpm".to_owned(), Value::from(self.request.tempo_bpm));
        data.insert("tempoSource".to_owned(), Value::from(self.tempo_source));
        data.insert(
            "quantization".to_owned(),
            serde_json::to_value(self.request.quantization).unwrap_or(Value::Null),
        );
        data.insert(
            "timeSignature".to_owned(),
            serde_json::json!({
                "numerator": self.request.time_signature.numerator,
                "denominator": self.request.time_signature.denominator,
            }),
        );
        data.insert("host".to_owned(), Value::from("in-process"));
        data.insert(
            "note".to_owned(),
            Value::from(
                "已交给宿主；引擎在量化点生效（本响应不代表已生效）。\
                 生效读数见 yeban_query_engine_state 的 engine.sceneLaunchesApplied。",
            ),
        );
        Value::Object(data)
    }
}

/// 从活跃工程解析 `sceneId`。
///
/// # Errors
///
/// - 不是 26 字符 ULID ⇒ `INVALID_PARAMETER_RANGE`（`reason: "invalidSceneId"`）；
/// - 合法但不在 `project.scenes` 里 ⇒ `ENTITY_NOT_FOUND`（`reason: "sceneNotFound"`，
///   带 `sceneCount`；⛔ **不**回落第一个场景）；
/// - 场景 tempo 覆盖非有限 / 越出模型区间 ⇒ `OUT_OF_RANGE`（`reason: "sceneTempoOutOfRange"`）。
pub fn resolve(project: &YebanProjectV1, scene_id_text: &str) -> Result<ResolvedScene, Fault> {
    let scene_id = EntityId::from_str(scene_id_text).map_err(|error| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`sceneId` 不是合法 ULID: {error}"),
            serde_json::json!({
                "reason": "invalidSceneId",
                "sceneId": scene_id_text,
                "hint": "`sceneId` 是 26 字符 ULID（与 yeban_query_project 的 entities[] 里 kind == \"scene\" 的 id 同形）",
            }),
        )
    })?;
    let Some(scene_index) = project.scenes.keys().position(|id| *id == scene_id) else {
        return Err(Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!(
                "场景 `{scene_id_text}` 不在当前工程里（共 {} 个场景）",
                project.scenes.len()
            ),
            serde_json::json!({
                "reason": "sceneNotFound",
                "sceneId": scene_id_text,
                "sceneCount": project.scenes.len(),
                "hint": "用 yeban_query_project 的 entities[] 读 kind == \"scene\" 的 id；⛔ 本工具不会回落到第一个场景",
            }),
        ));
    };
    let scene = project.scenes.get(&scene_id).expect("下标来自同一张表");
    let (tempo_bpm, tempo_source) = match scene.tempo {
        Some(tempo) => (tempo, "sceneOverride"),
        None => (project.bpm, "project"),
    };
    if !tempo_bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&tempo_bpm) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("场景启动的 tempo 必须在 {MIN_BPM:?}..={MAX_BPM:?} 且有限（实测 {tempo_bpm}）"),
            serde_json::json!({
                "reason": "sceneTempoOutOfRange",
                "sceneId": scene_id_text,
                "tempoBpm": tempo_bpm,
            }),
        ));
    }
    Ok(ResolvedScene {
        request: SceneLaunchRequest {
            scene_id,
            tempo_bpm,
            // 只读工程配置；⛔ 本模块不发明第二套量化档。
            quantization: project.transport.launch_quantization,
            time_signature: project.time_signature,
        },
        scene_index,
        scene_name: scene.name.clone(),
        tempo_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_scene() -> (YebanProjectV1, EntityId) {
        let mut project = yeban_model::samples::filled_project();
        let bpm = project.bpm;
        let (id, scene) = project.scenes.iter_mut().next().expect("样本必须有场景");
        let id = *id;
        scene.tempo = Some(bpm + 17.0);
        (project, id)
    }

    #[test]
    fn resolve_carries_the_scene_tempo_and_project_quantisation() {
        let (project, id) = project_with_scene();
        let resolved = resolve(&project, &id.to_canonical_string()).expect("解析");
        assert_eq!(resolved.request.scene_id, id);
        assert_eq!(resolved.request.tempo_bpm, project.bpm + 17.0);
        assert_eq!(resolved.tempo_source, "sceneOverride");
        assert_eq!(
            resolved.request.quantization,
            project.transport.launch_quantization
        );
        assert_eq!(resolved.request.time_signature, project.time_signature);
        let data = resolved.data(false);
        assert_eq!(data["queued"], false);
        assert_eq!(data["applied"], false);
        assert_eq!(data["sceneId"], id.to_canonical_string());
    }

    #[test]
    fn a_scene_without_a_tempo_override_follows_the_project() {
        let (mut project, id) = project_with_scene();
        project.scenes.get_mut(&id).expect("场景").tempo = None;
        let resolved = resolve(&project, &id.to_canonical_string()).expect("解析");
        assert_eq!(resolved.tempo_source, "project");
        assert_eq!(resolved.request.tempo_bpm, project.bpm);
    }

    /// 场景 tempo 的四值边界：19 / 20 / 999 / 1000（＋ 非有限）逐值同结论。
    ///
    /// 修前注入（实测，本单）：只把 `scene_launch.rs:157` 的 `999.0` 改成
    /// `998.0` ⇒ 全套 39 个 MCP test 二进制 989 passed / 0 failed 全绿。
    #[test]
    fn scene_tempo_bounds_follow_the_model_constants() {
        for (bpm, accepted) in [
            (MIN_BPM - 1.0, false),
            (MIN_BPM, true),
            (MAX_BPM, true),
            (MAX_BPM + 1.0, false),
            (f64::NAN, false),
        ] {
            let (mut project, id) = project_with_scene();
            project.scenes.get_mut(&id).expect("场景").tempo = Some(bpm);
            let result = resolve(&project, &id.to_canonical_string());
            assert_eq!(result.is_ok(), accepted, "scene tempo = {bpm}");
            if let Err(error) = result {
                match error {
                    Fault::Domain {
                        code,
                        message,
                        data,
                    } => {
                        assert_eq!(code, ErrorCode::OutOfRange, "{bpm}");
                        let data = data.expect("带结构化 data");
                        assert_eq!(data["reason"], "sceneTempoOutOfRange");
                        assert_eq!(data["sceneId"], id.to_canonical_string());
                        if bpm.is_finite() {
                            assert_eq!(
                                message.as_str(),
                                format!(
                                    "场景启动的 tempo 必须在 {MIN_BPM:?}..={MAX_BPM:?} 且有限（实测 {bpm}）"
                                )
                            );
                        }
                    }
                    other => panic!("必须是领域错误: {other:?}"),
                }
            }
        }
    }

    /// 发布文案逐字：`20.0..=999.0` 与 `#330` 之前的字节完全相同。
    #[test]
    fn scene_tempo_out_of_range_message_keeps_its_published_bytes() {
        let (mut project, id) = project_with_scene();
        project.scenes.get_mut(&id).expect("场景").tempo = Some(MAX_BPM + 1.0);
        let error = resolve(&project, &id.to_canonical_string()).expect_err("1000 必须被拒");
        let Fault::Domain {
            code,
            message,
            data,
            ..
        } = error
        else {
            panic!("必须是领域失败");
        };
        assert_eq!(code, ErrorCode::OutOfRange);
        assert_eq!(
            message,
            "场景启动的 tempo 必须在 20.0..=999.0 且有限（实测 1000）"
        );
        assert_eq!(data.expect("带结构化 data")["tempoBpm"], 1000.0);
    }

    #[test]
    fn an_unknown_scene_id_is_loud_and_never_falls_back_to_the_first_scene() {
        let (project, _) = project_with_scene();
        let ghost = "01J8ZQ00000000000000000000";
        let error = resolve(&project, ghost).expect_err("未知场景必须被拒");
        match error {
            Fault::Domain { code, data, .. } => {
                assert_eq!(code, ErrorCode::EntityNotFound);
                let data = data.expect("带结构化 data");
                assert_eq!(data["reason"], "sceneNotFound");
                assert_eq!(data["sceneId"], ghost);
                assert_eq!(data["sceneCount"], project.scenes.len());
            }
            other => panic!("必须是领域错误: {other:?}"),
        }
    }

    #[test]
    fn a_non_ulid_scene_id_is_invalid_parameter_range() {
        let (project, _) = project_with_scene();
        let error = resolve(&project, "not-a-ulid").expect_err("非法身份必须被拒");
        match error {
            Fault::Domain { code, data, .. } => {
                assert_eq!(code, ErrorCode::InvalidParameterRange);
                assert_eq!(data.expect("data")["reason"], "invalidSceneId");
            }
            other => panic!("必须是领域错误: {other:?}"),
        }
    }
}
