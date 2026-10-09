//! `yeban_propose_section` 的**契约适配层** [MCP-TOOL-005]。
//!
//! ## 这一层只做两件事
//!
//! 1. 把 [`super::section_build`] 的领域失败（[`BuildFault`]）映射成契约失败（[`Fault`]）；
//! 2. 把 `Op` 序列化成 `dryRun` 预览与提交记录共用的 JSON 形状。
//!
//! 生成逻辑本身住在 [`super::section_build`]（**零重依赖**：只用 `yeban-model` + `serde_json`），
//! 因为 `yeban-mcp` 传递性地依赖 `yeban-render` / `yeban-decode`，本机编不动整个 crate；
//! 抽出去之后那部分能在本机用裸 `rustc --edition 2024 --test` **真的**跑
//! （脚手架 `crates/yeban-mcp/verify/section_pure.rs`，与 `ADR-0001` D28 第 1 条同一手法）。
//!
//! ## 曾经写在这里的两条**过期假设**（已删，`ADR-0001` D43）
//!
//! 本模块头曾经断言"`Op` 全集没有 `AddClip`/`AddRoutingNode`，因此配器骨架里没有片段、
//! 声部连接无法表达"，并在响应里上报
//! `unwired = ["clipPoolEntries","routingEdges"]`。这两句话在
//! `ADR-0001` **D27**（= `HD-12`，2026-10-04 追认，`Op` 23 → 27）之后**已经不成立**：
//! 四个变体都在 `crates/yeban-model/src/ops.rs` 里，§7.2 的两种能力都能表达。
//! 过期的自我限制会主动误导下一个读者（Agent 会相信 `unwired` 然后绕开 `Op` 日志），
//! 因此按 D43"没有兼容包袱 ⇒ 旧假设该删就删"处理：断言删掉、`unwired` 改为
//! **由真实 op 推导**（见 [`super::section_build::unwired_for_section_op_kinds`]）。
//!
//! ## `CYCLE_DETECTED` 在哪里
//!
//! 模型层**不做**环路判定（`RoutingGraph::validate` 只查节点存在性与重复）。
//! [`super::section_build::detect_cycle`] 补上它（真 DFS 着色判环），
//! `plan` 在创建骨架**之前**先检查现有路由图 —— 在一个已经成环的工程上再叠加配器
//! 只会掩盖问题。

use serde_json::Value;

use yeban_model::{Op, YebanProjectV1};

use super::error::{Fault, from_model};
use super::section_build::{BuildCode, BuildFault};
use crate::tools::ErrorCode;

pub use super::section_build::{
    MAX_BARS, MAX_PARTS, NOTE_NAMES, NOTE_PITCH_CLASSES, Scale, SectionPlan, available_presets,
    detect_cycle, mode_names, parse_scale, preset_parts, ticks_per_bar,
    unwired_for_section_op_kinds,
};

/// [`BuildCode`] → 契约错误码（`ADR-0001` D25 的 20 值联集，**不发明新码**）。
///
/// 穷举匹配：`section_build` 新增一个失败类别时这里会**编译失败**，
/// 而不是悄悄落进某个兜底码。
#[must_use]
pub const fn error_code(code: BuildCode) -> ErrorCode {
    match code {
        BuildCode::StyleNotFound => ErrorCode::StyleNotFound,
        BuildCode::CycleDetected => ErrorCode::CycleDetected,
        BuildCode::OutOfRange => ErrorCode::OutOfRange,
        BuildCode::InvalidParameterRange => ErrorCode::InvalidParameterRange,
        BuildCode::Conflict => ErrorCode::Conflict,
        BuildCode::ClipNotFound => ErrorCode::ClipNotFound,
        BuildCode::TrackNotFound => ErrorCode::TrackNotFound,
    }
}

/// 规划一个章节骨架（真实生成逻辑见 [`super::section_build::plan`]）。
///
/// # Errors
///
/// - 未知风格预设 → `STYLE_NOT_FOUND`；
/// - `bars` 为 0 或超过 [`MAX_BARS`] → `OUT_OF_RANGE`；`scale` 写法非法 →
///   `INVALID_PARAMETER_RANGE`；
/// - 现有路由图已经成环 → `CYCLE_DETECTED`；
/// - 没有主总线音轨 → `TRACK_NOT_FOUND`；
/// - `clip_pool` 里没有可用材料（MIDI 且至少一个音符）→ `CLIP_NOT_FOUND`，
///   `data.missing = "usableClipPoolEntries"` 说明缺什么；
/// - 施加到克隆体后模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn plan(
    project: &YebanProjectV1,
    section_name: &str,
    style_preset: &str,
    bars: u64,
    scale: Option<&str>,
) -> Result<SectionPlan, Fault> {
    super::section_build::plan(project, section_name, style_preset, bars, scale).map_err(|fault| {
        match fault {
            BuildFault::Domain {
                code,
                message,
                data,
            } => Fault::domain_with_data(error_code(code), message, data),
            BuildFault::Model { context, error } => from_model(context, &error),
        }
    })
}

/// 一组 `Op` 的 JSON 列表（`dryRun` 预览与提交记录共用同一份形状）。
#[must_use]
pub fn ops_to_value(ops: &[Op]) -> Value {
    Value::Array(
        ops.iter()
            .map(|op| serde_json::to_value(op).unwrap_or(Value::Null))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    // 显式再列一遍父模块的**私有** `use`：`BuildCode` 住在兄弟模块 `domain::section_build`，
    // 而 `section` 自己在文件顶部只做了一个私有 `use`（跨模块可见性上显式更稳）。
    use super::super::section_build::BuildCode;
    use yeban_model::samples::filled_project;

    use crate::tools::ErrorCode;

    #[test]
    fn unknown_preset_is_style_not_found() {
        let project = filled_project();
        // `polka` **是** theory 的流派 ID（`GenreLibrary::get("polka")` 成功），
        // 所以它已经不是"未知预设"的例子了 —— 用一个 theory 里真的不存在的名字。
        let fault = plan(&project, "Chorus", "yeban_unknown_style", 8, None).expect_err("未知预设");
        assert_eq!(fault.domain_code(), Some(ErrorCode::StyleNotFound));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "STYLE_NOT_FOUND");
        assert!(value["error"]["data"]["availablePresets"].is_array());
        assert_eq!(
            value["error"]["data"]["stylePresetSource"],
            "yeban-theory::genre::GenreLibrary::ids"
        );
    }

    #[test]
    fn bars_and_scale_are_validated() {
        let project = filled_project();
        let fault = plan(&project, "Chorus", "lo_fi_hip_hop", 0, None).expect_err("bars=0");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let fault =
            plan(&project, "Chorus", "lo_fi_hip_hop", MAX_BARS + 1, None).expect_err("bars 过大");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let fault =
            plan(&project, "Chorus", "lo_fi_hip_hop", 4, Some("H dorian")).expect_err("音名");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let fault =
            plan(&project, "Chorus", "lo_fi_hip_hop", 4, Some("C bogus")).expect_err("调式");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        plan(&project, "Chorus", "lo_fi_hip_hop", 4, Some("c MINOR")).expect("大小写不敏感");
    }

    /// 缺材料必须是**契约内**的 `CLIP_NOT_FOUND`，且 `data` 说明缺什么。
    #[test]
    fn missing_material_is_clip_not_found() {
        let mut project = filled_project();
        project.clip_pool.clear();
        let fault = plan(&project, "Chorus", "lo_fi_hip_hop", 4, None).expect_err("无材料");
        assert_eq!(fault.domain_code(), Some(ErrorCode::ClipNotFound));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "CLIP_NOT_FOUND");
        assert_eq!(value["error"]["data"]["missing"], "usableClipPoolEntries");
        assert!(value["error"]["data"]["why"].is_string());
    }

    #[test]
    fn a_cyclic_project_is_refused_before_arranging() {
        use super::super::ids::deterministic_id;
        use yeban_model::{RoutingEdge, RoutingKind};

        let mut project = filled_project();
        let a = deterministic_id("cyc-a");
        let b = deterministic_id("cyc-b");
        project.routing_graph.nodes.push(a);
        project.routing_graph.nodes.push(b);
        for (source, destination) in [(a, b), (b, a)] {
            let edge = RoutingEdge {
                id: deterministic_id(&format!("edge:{source}:{destination}:cyc")),
                source_node: source,
                destination_node: destination,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            };
            project.routing_graph.edges.insert(edge.id, edge);
        }
        project.validate().expect("模型层不判环, 因此它是'合法'的");
        let fault = plan(&project, "Chorus", "lo_fi_hip_hop", 4, None).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::CycleDetected));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "CYCLE_DETECTED");
        assert!(value["error"]["data"]["cycle"].is_array());
    }

    /// 生成的骨架必须真的带片段与声部连接，且 `unwired` 为空 ——
    /// 这条判据是"过期假阻塞已删"的机械保护（把 `unwired` 加回来就红）。
    #[test]
    fn the_plan_wires_clips_and_routing_and_reports_no_unwired() {
        let project = filled_project();
        let planned = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        assert_eq!(planned.part_clip_ids.len(), planned.part_track_ids.len());
        assert_eq!(planned.placement_ids.len(), planned.part_track_ids.len());
        assert_eq!(planned.routing_edge_ids.len(), planned.part_track_ids.len());
        assert!(
            planned.unwired.is_empty(),
            "片段池与声部连接都已接线: {:?}",
            planned.unwired
        );
        let kinds: Vec<&str> = planned.ops.iter().map(Op::name).collect();
        for required in [
            "AddClip",
            "AddTrack",
            "AddClipPlacement",
            "AddRoutingNode",
            "ConnectRouting",
        ] {
            assert!(kinds.contains(&required), "缺少 {required}: {kinds:?}");
        }
    }

    /// 每一个 `BuildCode` 都必须映射进 `ADR-0001` D25 的契约 enum（不许发明新码）。
    #[test]
    fn every_build_code_maps_into_the_contract_enum() {
        assert_eq!(BuildCode::ALL.len(), 7, "类别清单与映射必须同步维护");
        for code in BuildCode::ALL {
            let mapped = error_code(code);
            assert!(
                ErrorCode::SCHEMA_CONTRACT.contains(&mapped),
                "{code:?} → {mapped:?} 不在契约 enum 里"
            );
        }
    }

    #[test]
    fn ops_to_value_keeps_every_payload() {
        let project = filled_project();
        let planned = plan(&project, "Chorus", "lo_fi_hip_hop", 2, None).expect("规划");
        let value = ops_to_value(&planned.ops);
        let array = value.as_array().expect("数组");
        assert_eq!(array.len(), planned.ops.len());
        assert_eq!(array[0]["SetSection"]["new_section"]["name"], "Chorus");
        // 逆操作同样逐条可序列化（提案审查者拿到的是同一份形状）。
        let mut simulated = project.clone();
        Op::Batch {
            ops: planned.ops.clone(),
            description: "判据".to_owned(),
        }
        .apply(&mut simulated)
        .expect("施加");
        Op::Batch {
            ops: planned.ops,
            description: "判据".to_owned(),
        }
        .apply_inverse(&mut simulated)
        .expect("逆操作");
        assert_eq!(
            serde_json::to_string(&simulated).expect("序列化"),
            serde_json::to_string(&project).expect("序列化")
        );
    }
}
