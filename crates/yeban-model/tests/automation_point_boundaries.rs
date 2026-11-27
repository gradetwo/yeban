//! **T-FIX-948 判据**：自动化点的写入、读取、插值、删除在边界上的**一致性**。
//!
//! 三条判据各自钉住一格既有判据没有覆盖的事实：
//!
//! 1. **重复点删除**：同一 tick 上两个点由 `(tick, id)` 最大者胜；删掉胜者后，
//!    同一 tick 的败者必须立刻接管，**前后两段的插值也随之改写**（不是只换命中值）。
//! 2. **时间轴端点**：`tick = 0` 与 `tick = u64::MAX` 都能写、读、删；删掉端点后
//!    求值改由相邻点保持，删空后隐式泳道被回收，工程回到逐字节原状。
//! 3. **目标越界**：`SetAutomationPoint` 与 MCP 写入层同一口径，目标必须先
//!    在文档里对账；不存在/越界的设备槽、参数、宏、路由边、CC 控制器必须**响亮**
//!    拒绝，⛔ 不得写出任何消费者都读不到的“幽灵泳道”。非有限点值同样拒绝。
//!
//! 为什么值得单独钉：修前 `SetAutomationPoint` 只查音轨存在，`DeviceParam` 的
//! 槽/参数、`SendGain` 的边、`Macro` 的下标、`MidiCc` 的控制器都**不查** ——
//! 写入会成功，`YebanProjectV1::validate()` 也放行，但唯一求值入口随后报目标
//! 不存在，界面按轨遍历却仍会画出这条幽灵曲线：同一条泳道在两条消费路径上
//! 成为两件事，且没有任何一步报错。

use std::str::FromStr;

use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveType, EntityId, ModelError, Op,
    TrackKind, YebanProjectV1,
};

fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

fn point(index: u128, tick: u64, value: f32) -> AutomationPoint {
    AutomationPoint {
        id: id(index),
        tick,
        value,
        curve: CurveType::Linear,
    }
}

fn lane(target: AutomationTarget, points: Vec<AutomationPoint>) -> AutomationLane {
    AutomationLane {
        target,
        points: points.into_iter().map(|p| (p.id, p)).collect(),
        ..AutomationLane::implicit(target)
    }
}

/// 取样本里第一条带设备的 MIDI 轨与它的音量目标。
fn fixture() -> (YebanProjectV1, EntityId, AutomationTarget) {
    let doc = yeban_model::samples::filled_project();
    let track_id = {
        let track = doc
            .tracks
            .values()
            .find(|track| track.kind == TrackKind::Midi && !track.devices.is_empty())
            .expect("样本里必须有带设备的 MIDI 轨");
        track.id
    };
    let target = AutomationTarget::TrackVolume { track_id };
    (doc, track_id, target)
}

fn install_lane(doc: &mut YebanProjectV1, track_id: EntityId, automation: AutomationLane) {
    doc.tracks
        .get_mut(&track_id)
        .expect("轨必须存在")
        .automation_lanes
        .insert(automation.target, automation);
}

/// 判据 ①：同 tick 两个点，删除胜者后败者接管，插值同步改写。
#[test]
fn deleting_one_of_two_same_tick_points_rewrites_both_sides_of_the_segment() {
    let (mut doc, track_id, target) = fixture();
    let first = point(100, 0, -12.0);
    let loser = point(200, 960, -20.0);
    let winner = point(300, 960, -10.0);
    let last = point(400, 1920, 0.0);
    install_lane(
        &mut doc,
        track_id,
        lane(target, vec![first, loser, winner, last]),
    );
    let setup = serde_json::to_string(&doc).expect("serialize");

    // 修前事实：同 tick 胜者是 id 更大者 ⇒ 960 命中 -10，前一段 0→960 也以 -10 为终点。
    assert_eq!(
        doc.automation_value_at(&target, 960),
        Ok(Some(-10.0)),
        "960 必须命中同 tick 胜者"
    );
    assert_eq!(
        doc.automation_value_at(&target, 480),
        Ok(Some(-11.0)),
        "0→960 的线性中点是 (-12 + -10) / 2"
    );

    // 删除胜者：960 的端点值改由败者 -20 接管，**同一条区间**的中点必须变成 -16。
    let remove_winner = Op::RemoveAutomationPoint {
        target,
        point_id: winner.id,
        previous_point: winner,
    };
    remove_winner.apply(&mut doc).expect("删除同 tick 胜者");
    assert_eq!(
        doc.automation_value_at(&target, 960),
        Ok(Some(-20.0)),
        "胜者删除后 960 必须由败者接管"
    );
    assert_eq!(
        doc.automation_value_at(&target, 480),
        Ok(Some(-16.0)),
        "区间终点换人后中点必须同步改写，不能沿用旧胜者的 -10"
    );

    // 单点与批量两个入口必须逐位一致（删除后仍一致）。
    let ticks = [0_u64, 480, 960, 1440, 1920];
    let batch = doc.automation_values_at(&target, &ticks).expect("批量求值");
    for (tick, got) in ticks.iter().zip(&batch) {
        assert_eq!(
            got.map(f32::to_bits),
            doc.automation_value_at(&target, *tick)
                .expect("单点求值")
                .map(f32::to_bits),
            "tick {tick} 的批量/单点结果必须逐位相同"
        );
    }

    // 序列化往返后仍是同一份曲线（删除不是只在内存里生效）。
    let parsed: YebanProjectV1 =
        serde_json::from_str(&serde_json::to_string(&doc).expect("serialize"))
            .expect("deserialize");
    for tick in ticks {
        assert_eq!(
            parsed
                .automation_value_at(&target, tick)
                .expect("往返后求值")
                .map(f32::to_bits),
            doc.automation_value_at(&target, tick)
                .expect("求值")
                .map(f32::to_bits),
            "往返后 tick {tick} 必须逐位一致"
        );
    }

    // 再删败者：960 不再有点，值由前后两个点插值 ⇒ 中点 -6。
    let remove_loser = Op::RemoveAutomationPoint {
        target,
        point_id: loser.id,
        previous_point: loser,
    };
    remove_loser.apply(&mut doc).expect("删除同 tick 败者");
    assert_eq!(
        doc.automation_value_at(&target, 960),
        Ok(Some(-6.0)),
        "同 tick 两点都删掉后，960 必须回到 0→1920 区间的插值"
    );

    // 两次删除的逆操作按相反顺序施加，必须逐字节回到初始文档。
    remove_loser.apply_inverse(&mut doc).expect("撤销删败者");
    remove_winner.apply_inverse(&mut doc).expect("撤销删胜者");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        setup,
        "删除的逆操作必须逐字节还原重复点"
    );
}

/// 判据 ②：时间轴两端都能写；删除端点后求值改由相邻点保持；删空回收隐式泳道。
#[test]
fn timeline_endpoints_are_writable_readable_and_removable() {
    let (mut doc, track_id, target) = fixture();
    doc.tracks
        .get_mut(&track_id)
        .expect("轨必须存在")
        .automation_lanes
        .remove(&target);
    let empty = serde_json::to_string(&doc).expect("serialize");

    let at_zero = point(500, 0, -5.0);
    let at_max = point(501, u64::MAX, 3.0);
    let set_zero = Op::SetAutomationPoint {
        target,
        point_id: at_zero.id,
        old_point: None,
        new_point: at_zero,
    };
    let set_max = Op::SetAutomationPoint {
        target,
        point_id: at_max.id,
        old_point: None,
        new_point: at_max,
    };
    set_zero.apply(&mut doc).expect("写 tick 0");
    set_max.apply(&mut doc).expect("写 tick u64::MAX");
    assert_eq!(
        doc.automation_lane(&target)
            .map(|automation| automation.points.len()),
        Some(2),
        "两个端点必须都落进同一条泳道"
    );
    let both = serde_json::to_string(&doc).expect("serialize");

    assert_eq!(
        doc.automation_value_at(&target, 0),
        Ok(Some(-5.0)),
        "tick 0 必须读回写入值"
    );
    assert_eq!(
        doc.automation_value_at(&target, u64::MAX),
        Ok(Some(3.0)),
        "tick u64::MAX 必须读回写入值"
    );
    let ticks = [0_u64, u64::MAX / 2, u64::MAX - 1, u64::MAX];
    let batch = doc.automation_values_at(&target, &ticks).expect("批量求值");
    for (tick, got) in ticks.iter().zip(&batch) {
        assert_eq!(
            got.map(f32::to_bits),
            doc.automation_value_at(&target, *tick)
                .expect("单点求值")
                .map(f32::to_bits),
            "端点区间内 tick {tick} 的批量/单点结果必须逐位相同"
        );
    }

    // 删掉末端点：u64::MAX 仍可读，值改为首点保持（不是 None、不是 panic）。
    let remove_max = Op::RemoveAutomationPoint {
        target,
        point_id: at_max.id,
        previous_point: at_max,
    };
    remove_max.apply(&mut doc).expect("删除末端点");
    assert_eq!(
        doc.automation_value_at(&target, u64::MAX),
        Ok(Some(-5.0)),
        "末端点删除后必须由首点保持"
    );

    // 删掉最后一个点：隐式泳道必须被回收，读入口退回 None（下游据此用静态值）。
    let remove_zero = Op::RemoveAutomationPoint {
        target,
        point_id: at_zero.id,
        previous_point: at_zero,
    };
    remove_zero.apply(&mut doc).expect("删除最后一个点");
    assert_eq!(
        doc.automation_lane(&target),
        None,
        "隐式泳道被移空后必须回收"
    );
    assert_eq!(
        doc.automation_value_at(&target, 0),
        Ok(None),
        "没有泳道必须返回 None，而不是一个空曲线值"
    );
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        empty,
        "两点写入再删空必须逐字节回到没有泳道的原状"
    );

    // 逆操作恢复两点（再删空间留下的空泳道不影响逐字节还原）。
    remove_zero.apply_inverse(&mut doc).expect("撤销删首点");
    remove_max.apply_inverse(&mut doc).expect("撤销删末点");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        both,
        "端点写入/删除的逆操作必须逐字节还原"
    );
}

/// 判据 ③：目标越界必须在写入前响亮拒绝，非有限点值同理；合法端点写入必须放行。
#[test]
fn out_of_range_targets_and_non_finite_values_are_refused_before_any_write() {
    let (mut doc, track_id, volume) = fixture();
    let setup = serde_json::to_string(&doc).expect("serialize");

    let bad_targets = [
        (
            AutomationTarget::TrackVolume { track_id: id(999) },
            "ghost-track",
        ),
        (
            AutomationTarget::SendGain {
                track_id,
                edge_id: id(998),
            },
            "ghost-edge",
        ),
        (
            AutomationTarget::DeviceParam {
                track_id,
                slot_index: 9,
                param_index: 0,
            },
            "slot 越界",
        ),
        (
            AutomationTarget::DeviceParam {
                track_id,
                slot_index: 0,
                param_index: 9,
            },
            "param 越界",
        ),
        (
            AutomationTarget::Macro {
                track_id,
                macro_index: 9,
            },
            "macro 越界",
        ),
        (
            AutomationTarget::MidiCc {
                track_id,
                controller: 200,
            },
            "CC 越界",
        ),
    ];
    for (index, (target, label)) in bad_targets.into_iter().enumerate() {
        let sample = point(600 + index as u128, 480, -6.0);
        let error = Op::SetAutomationPoint {
            target,
            point_id: sample.id,
            old_point: None,
            new_point: sample,
        }
        .apply(&mut doc)
        .expect_err(&format!("{label} 必须在写入前拒绝"));
        assert!(
            matches!(
                error,
                ModelError::TrackNotFound { .. }
                    | ModelError::RoutingEdgeNotFound { .. }
                    | ModelError::DeviceSlotOutOfRange { .. }
                    | ModelError::ParamIndexOutOfRange { .. }
                    | ModelError::MacroIndexOutOfRange { .. }
                    | ModelError::AutomationTargetNotApplicable { .. }
            ),
            "{label}: 必须是具体的模型错误, 实际 {error:?}"
        );
        assert_eq!(
            doc.automation_lane(&target),
            None,
            "{label}: 拒绝后不得留下幽灵泳道"
        );
    }
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        setup,
        "所有越界目标都必须一位不改"
    );

    // 非有限点值同样必须在写入前拒绝（NaN / ±INF 都不是可序列化状态）。
    let volume_points_before = doc.automation_lane(&volume).map(|lane| lane.points.len());
    for (index, bad) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY]
        .into_iter()
        .enumerate()
    {
        let sample = point(700 + index as u128, 480, bad);
        let error = Op::SetAutomationPoint {
            target: volume,
            point_id: sample.id,
            old_point: None,
            new_point: sample,
        }
        .apply(&mut doc)
        .expect_err("非有限点值必须拒绝");
        assert!(
            matches!(error, ModelError::NonFiniteValue { .. }),
            "非有限点值必须报 NonFiniteValue, 实际 {error:?}"
        );
        assert_eq!(
            doc.automation_lane(&volume).map(|lane| lane.points.len()),
            volume_points_before,
            "非有限点值拒绝后不得改变既有泳道"
        );
    }

    // 合法的边界目标必须放行：音量域端点、宏下标 0、CC 127、设备末一个参数。
    let accepted = [
        (volume, -60.0),
        (
            AutomationTarget::Macro {
                track_id,
                macro_index: 0,
            },
            0.0,
        ),
        (
            AutomationTarget::MidiCc {
                track_id,
                controller: 127,
            },
            127.0,
        ),
        (
            AutomationTarget::DeviceParam {
                track_id,
                slot_index: 0,
                param_index: 1,
            },
            0.0,
        ),
    ];
    for (index, (target, value)) in accepted.into_iter().enumerate() {
        let sample = point(800 + index as u128, 480, value);
        Op::SetAutomationPoint {
            target,
            point_id: sample.id,
            old_point: None,
            new_point: sample,
        }
        .apply(&mut doc)
        .unwrap_or_else(|error| panic!("合法目标 {target:?} 必须放行, 实际 {error:?}"));
        assert_eq!(
            doc.automation_value_at(&target, 480),
            Ok(Some(value)),
            "合法边界目标写入后必须能读回"
        );
    }
}
