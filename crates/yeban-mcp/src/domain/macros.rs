//! `yeban_set_macro` 的宏旋钮与**级联平滑自动化展开** [MCP-TOOL-007]。
//!
//! ## 做了什么、哪一半是"半做"
//!
//! - **真做**：音轨存在性（`TRACK_NOT_FOUND`）、宏下标（`INDEX_OUT_OF_BOUNDS`）、
//!   宏值域（`OUT_OF_RANGE`）、`Op::SetMacro` 的 `old_val` 从文档读取、
//!   以及按 `MacroMapping` 逐条展开的级联自动化点 —— 全部在克隆体上先模拟再校验。
//! - **半做**：级联点的**物理量纲**。`MacroMapping` 只给出 `depth`（0.0..=1.0），
//!   参数的真实值域（dB / Hz / %）住在设备/参数层，而 `Op::SetParam` 与
//!   [`yeban_model::ParameterValue`] 都不携带 min/max。因此本模块把级联点写成
//!   **归一化值** `depth * macro_value`，并在响应里如实标注
//!   `"normalized": true` —— 值域解析接线到设备层之后必须把这一位改成 `false`
//!   （见 `docs/ledger/tools-domain-notes.md` 的未接线清单）。
//!
//! ## 级联的跨度 = **工程拍号**下的"一小节"
//!
//! "平滑落在一小节上"是这条策略的全部内容，因此那一小节必须由工程的
//! `time_signature` 算出（[`super::section_build::ticks_per_bar`]，与配器段落长度、
//! 种子摆放长度共用同一个函数），而不是一条写死 `4/4` 的常量：`4/4` 是 `3840` tick，
//! `3/4` 是 `2880` tick。缺省（`4/4`）逐字节等于常量时代的行为。

use serde_json::Value;

use yeban_model::{AutomationPoint, AutomationTarget, CurveType, EntityId, Op, YebanProjectV1};

use super::error::{Fault, from_model};
use super::ids::deterministic_id;
use super::section::ops_to_value;
use crate::tools::ErrorCode;

/// 一次已校验的宏规划（**只读计算的产物**）。
#[derive(Clone, Debug, PartialEq)]
pub struct MacroPlan {
    /// 目标音轨。
    pub track_id: EntityId,
    /// 宏下标。
    pub macro_index: usize,
    /// 修改前的宏位置。
    pub old_value: f32,
    /// 修改后的宏位置。
    pub new_value: f32,
    /// 级联展开的时间跨度（tick）：**工程拍号**下的一小节
    /// （[`super::section_build::ticks_per_bar`]；"平滑"落在一小节上是可听的最小单位）。
    ///
    /// 为什么不是常量：拍号可设之后，"一小节"就不再只有 `3840` 这一个值
    /// （`3/4` ⇒ `2880`）。写死一条只对 `4/4` 成立的跨度，会让本工具在别的拍号里
    /// 与它自己的文档（"一小节"）不符。
    pub cascade_ticks: u64,
    /// 展开出来的级联自动化点身份（顺序 = 映射顺序）。
    pub cascade_points: Vec<EntityId>,
    /// 将要施加的领域操作。
    pub ops: Vec<Op>,
}

impl MacroPlan {
    /// `dryRun` 预览用的 JSON。
    #[must_use]
    pub fn preview(&self) -> Value {
        serde_json::json!({
            "trackId": self.track_id.to_canonical_string(),
            "macroIndex": self.macro_index,
            "oldValue": self.old_value,
            "newValue": self.new_value,
            "cascadeTicks": self.cascade_ticks,
            "cascadePoints": self
                .cascade_points
                .iter()
                .map(EntityId::to_canonical_string)
                .collect::<Vec<_>>(),
            "normalized": true,
            "normalizedReason": "MacroMapping 只给 depth; 参数值域住在设备层 (未接线)",
            "ops": ops_to_value(&self.ops),
        })
    }
}

/// 规划一次宏调整（含级联自动化展开）。
///
/// # Errors
///
/// - 音轨不存在 → `TRACK_NOT_FOUND`；
/// - `macroIndex` 越界 → `INDEX_OUT_OF_BOUNDS`；
/// - `value` 非有限或不在 `0.0..=1.0` → `OUT_OF_RANGE`；
/// - 模拟施加后模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn plan(
    project: &YebanProjectV1,
    track_id: &EntityId,
    macro_index: usize,
    value: f32,
) -> Result<MacroPlan, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    if macro_index >= track.macros.len() {
        return Err(Fault::domain_with_data(
            ErrorCode::IndexOutOfBounds,
            format!(
                "宏下标 {macro_index} 越界: 音轨 {track_id} 只有 {} 个宏",
                track.macros.len()
            ),
            serde_json::json!({
                "trackId": track_id.to_canonical_string(),
                "macroIndex": macro_index,
                "macroCount": track.macros.len(),
            }),
        ));
    }
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("宏值必须在 0.0..=1.0 且有限, 实际 {value}"),
            serde_json::json!({ "value": value }),
        ));
    }

    let macro_parameter = &track.macros[macro_index];
    let old_value = macro_parameter.value;
    let mut ops = vec![Op::SetMacro {
        track_id: *track_id,
        macro_index,
        old_val: old_value,
        new_val: value,
    }];

    let mut cascade_points = Vec::with_capacity(macro_parameter.mappings.len() * 2);
    // "一小节"由**工程拍号**算出（与配器段落长度、种子摆放长度同一个函数）：
    // 早先这里是一条写死的 `4/4` 常量，拍号可设之后它就成了同一份算法里的第二个真相。
    let cascade_ticks = super::section_build::ticks_per_bar(project);
    for (mapping_index, mapping) in macro_parameter.mappings.iter().enumerate() {
        let target: AutomationTarget = mapping.target;
        // 级联的形状: 本小节起点 → 一小节后, S 曲线平滑过渡; 幅值按 mapping.depth 缩放。
        for (step, (tick, level)) in [
            (0_u64, old_value * mapping.depth),
            (cascade_ticks, value * mapping.depth),
        ]
        .into_iter()
        .enumerate()
        {
            let point_id = deterministic_id(&format!(
                "macro-cascade:{track_id}:{macro_index}:{mapping_index}:{step}"
            ));
            let old_point = track
                .automation_lanes
                .get(&target)
                .and_then(|lane| lane.points.get(&point_id))
                .copied();
            cascade_points.push(point_id);
            ops.push(Op::SetAutomationPoint {
                target,
                point_id,
                old_point,
                new_point: AutomationPoint {
                    id: point_id,
                    tick,
                    value: level,
                    curve: CurveType::SCurve,
                },
            });
        }
    }

    let mut simulated = project.clone();
    Op::Batch {
        ops: ops.clone(),
        description: format!("set_macro {track_id}#{macro_index}"),
    }
    .apply(&mut simulated)
    .map_err(|error| from_model("宏级联模拟", &error))?;
    simulated
        .validate()
        .map_err(|error| from_model("宏级联校验", &error))?;

    Ok(MacroPlan {
        track_id: *track_id,
        macro_index,
        old_value,
        new_value: value,
        cascade_ticks,
        cascade_points,
        ops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    /// 样本里那个带宏与映射的音轨（`filled_project` 的 lead）。
    fn track_with_macro(project: &YebanProjectV1) -> EntityId {
        project
            .tracks
            .values()
            .find(|track| !track.macros.is_empty())
            .expect("样本里必须有带宏的音轨")
            .id
    }

    #[test]
    fn unknown_track_is_track_not_found() {
        let project = filled_project();
        let fault = plan(&project, &deterministic_id("ghost-track"), 0, 0.5).expect_err("无音轨");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
    }

    #[test]
    fn macro_index_and_value_are_checked() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let fault = plan(&project, &track_id, 7, 0.5).expect_err("下标越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IndexOutOfBounds));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INDEX_OUT_OF_BOUNDS");
        assert_eq!(value["error"]["data"]["macroCount"], 1);

        for bad in [-0.5_f32, 1.5, f32::NAN, f32::INFINITY] {
            let fault = plan(&project, &track_id, 0, bad).expect_err("值域");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::OutOfRange),
                "{bad} 应当越界"
            );
        }
    }

    #[test]
    fn cascade_expands_every_mapping_and_is_deterministic() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let first = plan(&project, &track_id, 0, 0.25).expect("规划");
        let second = plan(&project, &track_id, 0, 0.25).expect("规划");
        assert_eq!(first, second, "同一请求必须产出同一份规划");
        // SetMacro + 每个映射 2 个级联点。
        assert_eq!(first.cascade_points.len(), 2);
        assert_eq!(first.ops.len(), 3);
        assert_eq!(first.old_value, 0.5);
        assert_eq!(first.new_value, 0.25);
        assert_eq!(
            first.cascade_ticks, 3_840,
            "4/4 样本的一小节 = 4 拍 × 960 PPQ"
        );
        match &first.ops[1] {
            Op::SetAutomationPoint {
                new_point, target, ..
            } => {
                assert_eq!(new_point.tick, 0);
                assert_eq!(new_point.curve, CurveType::SCurve);
                // 归一化: depth(0.8) * old(0.5) = 0.4
                assert!((new_point.value - 0.4).abs() < 1e-6, "{}", new_point.value);
                assert!(matches!(target, AutomationTarget::DeviceParam { .. }));
            }
            other => panic!("应当是 SetAutomationPoint: {other:?}"),
        }

        let mut simulated = project.clone();
        Op::Batch {
            ops: first.ops.clone(),
            description: "t".to_owned(),
        }
        .apply(&mut simulated)
        .expect("施加");
        simulated.validate().expect("施加后必须合法");
    }

    /// 级联跨度 = **工程拍号**下的一小节（不是写死的 `4/4`）。
    #[test]
    fn cascade_span_follows_the_project_time_signature() {
        let mut project = filled_project();
        project.time_signature = yeban_model::TimeSignature {
            numerator: 3,
            denominator: 4,
        };
        let track_id = track_with_macro(&project);
        let planned = plan(&project, &track_id, 0, 0.25).expect("规划");
        assert_eq!(
            planned.cascade_ticks, 2_880,
            "3/4 的一小节 = 3 拍 × 960 PPQ"
        );
        // 终点两点真的落在那个跨度上（不是 4/4 的 3840）。
        let ticks: Vec<u64> = planned
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::SetAutomationPoint { new_point, .. } => Some(new_point.tick),
                _ => None,
            })
            .collect();
        assert_eq!(ticks, vec![0, 2_880]);
        assert_eq!(planned.preview()["cascadeTicks"], serde_json::json!(2_880));
        // 施加之后文档必须仍然合法（跨度变了不该让 `Op` 失效）。
        let mut simulated = project.clone();
        Op::Batch {
            ops: planned.ops.clone(),
            description: "t".to_owned(),
        }
        .apply(&mut simulated)
        .expect("施加");
        simulated.validate().expect("施加后必须合法");
    }

    #[test]
    fn re_applying_the_same_macro_plan_is_stable() {
        // 第二遍时 `old_point` 已经是 Some, 载荷必须随文档更新（否则模型层会拒绝）。
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let first = plan(&project, &track_id, 0, 0.5).expect("规划");
        let mut after = project.clone();
        Op::Batch {
            ops: first.ops.clone(),
            description: "t".to_owned(),
        }
        .apply(&mut after)
        .expect("施加");
        let second = plan(&after, &track_id, 0, 0.9).expect("再规划");
        assert_eq!(second.old_value, 0.5, "旧值必须来自当前文档");
        let mut again = after.clone();
        Op::Batch {
            ops: second.ops.clone(),
            description: "t".to_owned(),
        }
        .apply(&mut again)
        .expect("再施加");
        again.validate().expect("合法");
        // 逆操作: 回到第一遍之后的状态。
        for op in second.ops.iter().rev() {
            op.apply_inverse(&mut again).expect("可逆");
        }
        assert_eq!(again, after, "级联必须是可逆的");
    }
}
