//! `yeban_set_macro` 的宏旋钮与**级联平滑自动化展开** [MCP-TOOL-007]。
//!
//! ## 级联点的**真实量纲**（任务单 `#297`）
//!
//! 级联点写的是**目标原生单位**，公式逐字为
//!
//! ```text
//! 获得值 = 目标当前值 + depth × macro_value
//! ```
//!
//! - `macro_value` 是宏位置（`0.0..=1.0`）；
//! - `depth` 是**目标单位的振幅**（`MacroMapping::depth`）：增益是 dB、声相是位置、
//!   设备参数是它自己的单位。`depth = 6.0` 的增益映射走到 `macro_value = 1.0`
//!   ⇒ 相对目标当前值 `+6 dB`（⛔ 不再是 `depth × value` 的归一化标量）；
//! - **基线** = 目标的当前值（[`AutomationTarget::static_value`]）。`DeviceParam`
//!   没有单独的"设备默认值"字段，它的当前 `ParameterValue.value` 就是基线
//!   （⛔ 不是设备出厂默认值）—— 这一点在响应里也如实写出。
//!
//! 级联点仍然**物化**进 `mapping.target` 的泳道：离线渲染器消费目标泳道的既有路径
//! 不变。另一条动态路径（`AutomationTarget::Macro` 泳道）只在**存在该泳道**时才
//! 级联，因此没有宏泳道的工程不会与这里的物化双计。⚠ 同一个宏**同时**有 `Macro`
//! 泳道与物化点时，两者会叠加一次（本单未去重；现有夹具无此组合）。
//!
//! ## 级联的跨度 = **工程拍号**下的"一小节"
//!
//! "平滑落在一小节上"是这条策略的全部内容，因此那一小节必须由工程的
//! `time_signature` 算出（[`super::section_build::ticks_per_bar`]，与配器段落长度、
//! 种子摆放长度共用同一个函数），而不是一条写死 `4/4` 的常量：`4/4` 是 `3840` tick，
//! `3/4` 是 `2880` tick。缺省（`4/4`）逐字节等于常量时代的行为。
//!
//! ## 同值重放不得改写级联的起点（类别⑤ 幂等性）
//!
//! 级联起点（tick 0）的幅值是 `目标当前值 + old_value × depth`，它的意思是"参数
//! **此刻**在哪"。旋钮**没有移动**时（`old_value` 与本次请求的目标逐位同值）这条公式
//! 会让第二次调用把它**自己在第一次调用里写下**的起点改写掉：一次 `set_macro(0.33)`
//! 从 0.5 落下时写下的是 `静态值 + 0.5 × depth`，紧接着再来一次同参调用会把它改成
//! `静态值 + 0.33 × depth` ⇒ 那一小节的斜坡被压平，工程 digest 变了 ——
//! 同一份请求第二次改变了文档。`Op::SetMacro` 的注释把"值没变要不要提交"交给
//! **调用方**决定（`crates/yeban-model/src/ops.rs`：`new_* == old_*` 是合法的无操作），
//! 而这里是那个调用方：因此同值时**保留文档现值**（有既有起点就用它，没有才退化成平线），
//! 只有旋钮真的移动过才按 `目标当前值 + old_value × depth` 重算起点。
//! 判据 `same_value_replay_leaves_the_document_byte_identical`（模块内）与
//! `set_macro_same_value_replay_leaves_the_project_byte_identical`（`tests/tools_e2e.rs`）
//! 钉住这条；`a_changed_value_still_ramps_from_the_current_knob_position` 钉住
//! 它**没有**把"旋钮移动过"的那条路一起改掉。

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
            "normalized": false,
            "normalizedReason": "级联点已是目标原生单位: 目标当前值 + depth × macroValue（depth 是目标单位的振幅；DeviceParam 的基线是当前参数值，不是设备默认值）",
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
    // 旋钮**有没有真的移动**（逐位比较，与模型 `same_f32` 同口径：`-0.0` 与 `0.0` 不是同一个值）。
    // 为什么它决定级联起点：起点的幅值 `基线 + old_value * depth` 是"参数此刻在哪"。
    // 同值重放时旋钮没有移动 ⇒ 若照此重算，第二次调用会把它自己在第一次调用里写下的那个
    // 起点**改写**掉（一小节的斜坡被压平）—— 同一份请求第二次改变了文档，类别⑤ 不成立。
    // 因此同值时保留文档现值（既有起点），既不改写自己写的，也不覆盖别人写的自动化。
    let knob_moved = old_value.to_bits() != value.to_bits();
    for (mapping_index, mapping) in macro_parameter.mappings.iter().enumerate() {
        let target: AutomationTarget = mapping.target;
        // `#297`：基线 = 目标的**当前值**（`DeviceParam` 的当前 `ParameterValue.value`
        // 就是能拿到的"默认值"；模型里没有单独的出厂默认字段）。
        let baseline = target
            .static_value(project)
            .map_err(|error| from_model("宏级联基线", &error))?;
        // 级联的形状: 本小节起点 → 一小节后, S 曲线平滑过渡;
        // 幅值 = 基线 + mapping.depth × 宏位置（depth 是目标单位的振幅）。
        for (step, tick) in [0_u64, cascade_ticks].into_iter().enumerate() {
            let point_id = deterministic_id(&format!(
                "macro-cascade:{track_id}:{macro_index}:{mapping_index}:{step}"
            ));
            let old_point = track
                .automation_lanes
                .get(&target)
                .and_then(|lane| lane.points.get(&point_id))
                .copied();
            // 终点永远跟随本次请求的目标；起点只在旋钮**真的移动过**时才重算。
            let level = if step == 0 {
                if knob_moved {
                    baseline + old_value * mapping.depth
                } else {
                    old_point.map_or(baseline + old_value * mapping.depth, |point| point.value)
                }
            } else {
                baseline + value * mapping.depth
            };
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
        // `#297`：预览必须说真话 —— 级联点已是目标原生单位。
        assert_eq!(first.preview()["normalized"], serde_json::json!(false));
        assert!(
            first.preview()["normalizedReason"]
                .as_str()
                .is_some_and(|reason| reason.contains("目标当前值 + depth × macroValue")),
            "{}",
            first.preview()
        );
        match &first.ops[1] {
            Op::SetAutomationPoint {
                new_point, target, ..
            } => {
                assert_eq!(new_point.tick, 0);
                assert_eq!(new_point.curve, CurveType::SCurve);
                // `#297` 真实量纲: 目标当前值(1200.0) + depth(0.8) * old(0.5) = 1200.4
                assert!(
                    (new_point.value - 1_200.4).abs() < 1e-4,
                    "{}",
                    new_point.value
                );
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

    /// 施加一份规划（与 `domain::apply` 走同一个 `Op::Batch`），产出施加后的文档。
    fn applied(plan: &MacroPlan, project: &YebanProjectV1) -> YebanProjectV1 {
        let mut after = project.clone();
        Op::Batch {
            ops: plan.ops.clone(),
            description: "t".to_owned(),
        }
        .apply(&mut after)
        .expect("施加");
        after
    }

    /// 规划里 **tick 0** 那个级联点的幅值（第一个映射的起点；夹具只有一个映射）。
    fn cascade_start_level(plan: &MacroPlan) -> f32 {
        plan.ops
            .iter()
            .find_map(|op| match op {
                Op::SetAutomationPoint { new_point, .. } if new_point.tick == 0 => {
                    Some(new_point.value)
                }
                _ => None,
            })
            .expect("级联起点")
    }

    /// 夹具前提：样本音轨的宏值（`filled_project` 的 lead 是 `0.5`，一个映射
    /// `depth = 0.8`，目标 `DeviceParam(cutoff)` 的当前值是 `1200.0`）。
    fn fixture_macro(project: &YebanProjectV1, track_id: &EntityId) -> (f32, f32, f32) {
        let macro_parameter = &project.tracks[track_id].macros[0];
        let mapping = &macro_parameter.mappings[0];
        let baseline = mapping
            .target
            .static_value(project)
            .expect("夹具目标必须有当前值");
        (macro_parameter.value, mapping.depth, baseline)
    }

    /// **类别⑤（幂等性）**：旋钮没有移动时，同值重放**不得**改变文档。
    ///
    /// 改动前实测（真二进制 `yeban-mcp --stdio`，工程 = 模型样本容器）：
    /// 第一次 `set_macro(0.33)` 写下 DeviceParam 泳道的一小节斜坡（`#297` 起是
    /// `目标当前值 + 位置 × depth` 的真实单位；归一化时代的读数是 `0.4 → 0.264`）；
    /// 紧接着同参第二次若按公式重算，会把 `tick 0` 改写成终点值 ⇒ 斜坡被压平。
    /// （归一化时代的实测 digest 由 `eb637dbb…` 变成 `fa4c5905…`；`#297` 改了
    /// 级联单位，本判据钉的是**机制**：同值重放之后工程逐字节不变。）
    /// 本判据把"同一份请求第二次之后文档逐字段不变"钉住。
    #[test]
    fn same_value_replay_leaves_the_document_byte_identical() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let (macro_value, depth, baseline) = fixture_macro(&project, &track_id);

        // 第一次：旋钮真的移动（0.5 → 0.33）⇒ 建立斜坡，
        // 起点 = 基线 + 0.5 × depth（`#297` 的真实单位）。
        let down = plan(&project, &track_id, 0, 0.33).expect("规划");
        assert!((cascade_start_level(&down) - (baseline + macro_value * depth)).abs() < 1e-4);
        let once = applied(&down, &project);

        // 第二次：**同参**（旋钮没有移动）。
        let replay = plan(&once, &track_id, 0, 0.33).expect("规划");
        assert_eq!(replay.old_value, 0.33, "旧值必须来自当前文档");
        assert!(
            (cascade_start_level(&replay) - (baseline + macro_value * depth)).abs() < 1e-4,
            "同值重放必须保留既有起点 {baseline} + {macro_value} × {depth}"
        );
        assert!(
            (cascade_start_level(&replay) - (baseline + 0.33 * depth)).abs() > 1e-4,
            "起点不得被同值重放改写成 {baseline} + 0.33 × {depth}"
        );
        let twice = applied(&replay, &once);
        assert_eq!(twice, once, "同值重放之后文档必须逐字段不变");
    }

    /// 对照组：旋钮**真的移动过**时，起点仍然跟随**当前**旋钮位置（不是既有起点）。
    ///
    /// 没有这一条，`same_value_replay_leaves_the_document_byte_identical` 可以被
    /// "永远保留既有起点"这种过度修正骗过 —— 那会让第二次**换值**的落点从上一小节
    /// 的起点开始，而不是从当前旋钮位置开始。
    #[test]
    fn a_changed_value_still_ramps_from_the_current_knob_position() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let (_, depth, baseline) = fixture_macro(&project, &track_id);

        let down = plan(&project, &track_id, 0, 0.33).expect("规划");
        let once = applied(&down, &project);

        let up = plan(&once, &track_id, 0, 0.9).expect("规划");
        assert!(
            (cascade_start_level(&up) - (baseline + 0.33 * depth)).abs() < 1e-4,
            "旋钮移动过 ⇒ 起点 = 基线 + 当前旋钮位置 × depth"
        );
        let after = applied(&up, &once);
        assert_ne!(after, once, "换值必须真的改变文档");
    }

    /// 边界：同值且**没有**既有起点 ⇒ 退化成平线（起点 = 终点 = 基线 + 目标 × depth）。
    ///
    /// 夹具前提（实测）：`filled_project` 的 lead 只有 `TrackVolume` 泳道，
    /// `DeviceParam` 泳道不存在 ⇒ 起点没有现值可保留。
    #[test]
    fn same_value_without_an_existing_point_writes_a_flat_pair() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let (current, depth, baseline) = fixture_macro(&project, &track_id);
        let before = project.tracks[&track_id].automation_lanes.len();

        let flat = plan(&project, &track_id, 0, current).expect("规划");
        assert_eq!(flat.old_value, current);
        assert!(
            (cascade_start_level(&flat) - (baseline + current * depth)).abs() < 1e-4,
            "没有既有起点 ⇒ 起点退化成 基线 + 目标 × depth"
        );
        let after = applied(&flat, &project);
        assert_eq!(
            after.tracks[&track_id].automation_lanes.len(),
            before + 1,
            "平线仍然要真的落进文档（这是本工具做的事）"
        );
        // 第二遍同值 ⇒ 文档不动（与上一条判据同一结论的另一条路径）。
        let again = applied(
            &plan(&after, &track_id, 0, current).expect("再规划"),
            &after,
        );
        assert_eq!(again, after);
    }

    /// **边界**：`macroIndex` **恰好等于**宏个数时必须是 `INDEX_OUT_OF_BOUNDS`。
    ///
    /// 既有判据只喂了下标 `7`（宏个数是 `1`）⇒ `>=` 与 `>` 在那里同解。
    /// 这一格不同解的地方在**恰好越界一格**：`>` 会让它掉进
    /// `&track.macros[macro_index]`，那是数组越界 panic（进程级失败），
    /// 而工具面的契约是带内的 `INDEX_OUT_OF_BOUNDS`（`data.macroCount` 报出真实个数）。
    #[test]
    fn a_macro_index_equal_to_the_count_is_out_of_bounds_not_a_panic() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        let count = project.tracks[&track_id].macros.len();
        assert_eq!(count, 1, "夹具前提: 样本音轨恰好有一个宏");

        let fault = plan(&project, &track_id, count, 0.5).expect_err("恰好越界一格");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IndexOutOfBounds));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["macroCount"], count);
        // 反向: 合法的最后一个下标仍然必须通过（闸门不是"一律拒绝"）。
        plan(&project, &track_id, count - 1, 0.5).expect("最后一个合法下标");
    }

    /// **值域闸门是 MCP 层自己的**：`value` 有限但越界时，报文必须来自本模块的闸门，
    /// 而不是模拟施加后从模型层折回来的那一条。
    ///
    /// 两条路的**契约码相同**（都是 `OUT_OF_RANGE`，见 `error::code_for_model` 的
    /// `MacroValueOutOfRange` 那一臂），因此只断言码的判据区分不了它们；能区分的是
    /// `data`：本模块给 `{"value": …}` 与中文文案，模型那条给 `{"model": "<Debug>"}`。
    #[test]
    fn an_out_of_range_value_is_refused_here_not_by_the_model() {
        let project = filled_project();
        let track_id = track_with_macro(&project);
        for bad in [1.5_f32, -0.5] {
            let fault = plan(&project, &track_id, 0, bad).expect_err("有限但越界");
            assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange), "{bad}");
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["value"],
                serde_json::json!(bad),
                "{bad} 必须由本模块的值域闸门拒绝: {value}"
            );
            assert!(
                value["error"]["data"]["model"].is_null(),
                "{bad} 不得落到模型层（那说明闸门被绕过了）: {value}"
            );
            assert!(
                value["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("宏值必须在 0.0..=1.0 且有限")),
                "{bad}: {value}"
            );
        }
    }
}
