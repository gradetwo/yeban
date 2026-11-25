//! T-FIX-93 同批（2026-11-24）：`Op::SetMacroMappings` 判据。
//!
//! 钉四件事：
//! 1. **陈旧载荷必被拒** —— `old_mappings` 与文档现值逐项不符 ⇒ `OpStateMismatch`，文档一位不动；
//! 2. **合法替换逐位可逆** —— 应用后映射整体替换，逆操作（新旧互换）撤销后文档逐位回到之前；
//! 3. **非法深度先于落盘被拒** —— `depth` 非有限 ⇒ `NonFiniteValue`，文档一位不动；
//! 4. **空列表是一等状态** —— 清空全部映射合法（不是"丢失"），且同样可逆。
//!
//! 前提事实（T-FIX-93 任务单 + `bridge::MACRO_MAPPING_PANEL_NOT_WIRED` 旧锚，
//! 当时旧名 `MACRO_MAPPING_NO_WRITE_PATH`）：此前 `Op`
//! 全集没有任何变体能写 `MacroParameter::mappings` —— 本变体就是那个缺口的第一块落地
//! （model 层；MCP 工具面同批接线；GUI 宏映射面板的改绑入口由 T-FIX-88 的承接方接线）。

use yeban_model::ops::Op;
use yeban_model::project::{AutomationTarget, MacroMapping};
use yeban_model::samples;

/// 取夹具里第一个带宏的音轨（id, 宏下标恒 0）。
fn track_with_macro(project: &yeban_model::YebanProjectV1) -> yeban_model::EntityId {
    project
        .tracks
        .values()
        .find(|track| !track.macros.is_empty())
        .map(|track| track.id)
        .expect("filled_project 夹具必须带至少一个宏")
}

#[test]
fn stale_mappings_payload_is_rejected_and_document_untouched() {
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let before = doc.clone();

    let stale = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: Vec::new(), // ⛔ 与现值（夹具里有映射）不符
        new_mappings: Vec::new(),
    };
    assert!(
        matches!(
            stale.apply(&mut doc),
            Err(yeban_model::ModelError::OpStateMismatch { .. })
        ),
        "陈旧载荷必须报 OpStateMismatch"
    );
    assert_eq!(doc, before, "失败的 apply 绝不能改文档");
}

#[test]
fn valid_replacement_round_trips_bitwise() {
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let current = doc.track(&track_id).expect("lead").macros[0]
        .mappings
        .clone();
    let new_mappings = vec![MacroMapping {
        target: AutomationTarget::TrackVolume { track_id },
        depth: 6.0,
    }];
    let before = doc.clone();

    let op = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: current.clone(),
        new_mappings: new_mappings.clone(),
    };
    op.apply(&mut doc).expect("SetMacroMappings 必须可应用");
    assert_eq!(
        doc.track(&track_id).expect("lead").macros[0].mappings,
        new_mappings,
        "映射必须被整体替换"
    );
    // 宏位置不许被这个变体顺手动过（各写各的字段）。
    assert_eq!(
        doc.track(&track_id).expect("lead").macros[0].value,
        before.track(&track_id).expect("lead").macros[0].value
    );

    let inverse = op.invert(&doc).expect("求逆");
    assert_eq!(
        inverse,
        Op::SetMacroMappings {
            track_id,
            macro_index: 0,
            old_mappings: new_mappings,
            new_mappings: current,
        },
        "逆操作 = 新旧互换"
    );
    inverse.apply(&mut doc).expect("撤销");
    assert_eq!(doc, before, "宏映射替换必须逐位可逆");
}

#[test]
fn non_finite_depth_is_rejected_before_write() {
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let current = doc.track(&track_id).expect("lead").macros[0]
        .mappings
        .clone();
    let before = doc.clone();

    let bad = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: current,
        new_mappings: vec![MacroMapping {
            target: AutomationTarget::TrackVolume { track_id },
            depth: f32::NAN,
        }],
    };
    assert!(
        matches!(
            bad.apply(&mut doc),
            Err(yeban_model::ModelError::NonFiniteValue { field, .. })
                if field == "macro.mapping.depth"
        ),
        "非有限深度必须报 NonFiniteValue{{macro.mapping.depth}}"
    );
    assert_eq!(doc, before, "失败的 apply 绝不能改文档");
}

#[test]
fn dangling_target_is_rejected_by_precondition_and_document_untouched() {
    // T-FIX-254：映射 target 必须对工程文档对账，拒绝悬空目标。
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let current = doc.track(&track_id).expect("lead").macros[0]
        .mappings
        .clone();
    let before = doc.clone();

    // 一个不存在的 EntityId 作目标音轨 —— 悬空引用。
    let ghost = yeban_model::EntityId::new();
    let bad = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: current,
        new_mappings: vec![MacroMapping {
            target: AutomationTarget::TrackVolume { track_id: ghost },
            depth: 1.0,
        }],
    };
    assert!(
        matches!(
            bad.apply(&mut doc),
            Err(yeban_model::ModelError::TrackNotFound { .. })
        ),
        "悬空 track 目标必须报 TrackNotFound（由 validate_against 对账）"
    );
    assert_eq!(doc, before, "失败的 apply 绝不能改文档");
}

#[test]
fn dangling_device_slot_target_is_rejected() {
    // 另一类悬空：设备槽/参数下标越界。
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let current = doc.track(&track_id).expect("lead").macros[0]
        .mappings
        .clone();
    let before = doc.clone();

    let bad = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: current,
        new_mappings: vec![MacroMapping {
            target: AutomationTarget::DeviceParam {
                track_id,
                slot_index: usize::MAX,
                param_index: 0,
            },
            depth: 1.0,
        }],
    };
    assert!(
        matches!(
            bad.apply(&mut doc),
            Err(yeban_model::ModelError::DeviceSlotOutOfRange { .. })
        ),
        "越界设备槽目标必须报 DeviceSlotOutOfRange"
    );
    assert_eq!(doc, before, "失败的 apply 绝不能改文档");
}

#[test]
fn clearing_all_mappings_is_a_first_class_state_and_reversible() {
    let mut doc = samples::filled_project();
    let track_id = track_with_macro(&doc);
    let current = doc.track(&track_id).expect("lead").macros[0]
        .mappings
        .clone();
    assert!(
        !current.is_empty(),
        "filled_project 的宏必须带映射，否则本判据失去意义"
    );
    let before = doc.clone();

    let op = Op::SetMacroMappings {
        track_id,
        macro_index: 0,
        old_mappings: current,
        new_mappings: Vec::new(), // 清空 = 一等状态
    };
    op.apply(&mut doc).expect("清空映射必须可应用");
    assert!(
        doc.track(&track_id).expect("lead").macros[0]
            .mappings
            .is_empty(),
        "清空后映射必须是空列表"
    );
    op.invert(&doc)
        .expect("求逆")
        .apply(&mut doc)
        .expect("撤销");
    assert_eq!(doc, before, "清空必须逐位可逆（不是\"丢失\"）");
}
