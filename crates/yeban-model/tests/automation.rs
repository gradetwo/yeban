//! 自动化泳道的判据 [MODEL-AST-002, ARCH-DET-001, ARCH-OPS-001]。
//!
//! 本文件是本线（`line/model-automation`）交给下游三条线（渲染 / 引擎 / 界面）的
//! **可执行契约**：数据形状、求值口径（含全部边界）、两个新 `Op` 的真逆、
//! serde 往返、旧工程兼容、逐位确定性、与 `AutomationTarget` 的对账。
//!
//! 每条判据的编号与 `docs/ledger/model-automation-notes.md` 的判据表一一对应。

use std::collections::BTreeMap;
use std::str::FromStr;

use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationUnit, AutomationValueDomain,
    AutomationWriteMode, CurveType, DeviceDefinition, EntityId, MacroMapping, MacroParameter,
    ModelError, Op, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

/// 构造确定性的规范 ULID 文本（与 crate 内夹具同一口径）。
fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 一条采样点。
fn point(index: u128, tick: u64, value: f32, curve: CurveType) -> AutomationPoint {
    AutomationPoint {
        id: id(index),
        tick,
        value,
        curve,
    }
}

/// 用给定采样点构造泳道（读开、写 `Off`、无显式取值域）。
fn lane(target: AutomationTarget, points: Vec<AutomationPoint>) -> AutomationLane {
    AutomationLane {
        target,
        points: points.into_iter().map(|p| (p.id, p)).collect(),
        ..AutomationLane::implicit(target)
    }
}

/// 往泳道里塞一个采样点。
fn with_point(mut automation: AutomationLane, point: AutomationPoint) -> AutomationLane {
    automation.points.insert(point.id, point);
    automation
}

/// 两份端点固定的泳道（`t = 0.0` 处取 `0.0`，`t = 1.0` 处取 `1.0`），便于逐形状对账。
fn span_lane(target: AutomationTarget, curve: CurveType) -> AutomationLane {
    lane(
        target,
        vec![
            point(100, 0, 0.0, curve),
            point(101, 1000, 1.0, CurveType::Linear),
        ],
    )
}

/// 夹具：Master + 一条带设备/宏/路由边的 Lead。
struct Fixture {
    lead: EntityId,
    edge: EntityId,
    volume: AutomationTarget,
    pan: AutomationTarget,
    param: AutomationTarget,
    macro_target: AutomationTarget,
    send: AutomationTarget,
}

fn fixture() -> (YebanProjectV1, Fixture) {
    let master = id(1);
    let lead = id(2);
    let edge = id(30);
    let mut doc = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    doc.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    doc.tracks.insert(
        lead,
        TrackV3 {
            id: lead,
            name: "Lead".to_owned(),
            kind: TrackKind::Midi,
            volume_db: -3.0,
            pan: -0.25,
            devices: vec![DeviceDefinition {
                id: id(40),
                name: "PolySynth".to_owned(),
                params: vec![
                    ParameterValue {
                        name: "cutoff".to_owned(),
                        value: 1200.0,
                        unit: Some("Hz".to_owned()),
                    },
                    ParameterValue {
                        name: "reso".to_owned(),
                        value: 0.3,
                        unit: None,
                    },
                ],
                ..DeviceDefinition::default()
            }],
            macros: vec![MacroParameter {
                name: "Brightness".to_owned(),
                value: 0.5,
                mappings: vec![MacroMapping {
                    target: AutomationTarget::Macro {
                        track_id: lead,
                        macro_index: 0,
                    },
                    depth: 0.5,
                }],
            }],
            ..TrackV3::default()
        },
    );
    doc.routing_graph = RoutingGraph {
        nodes: vec![master, lead],
        edges: BTreeMap::from([(
            edge,
            RoutingEdge {
                id: edge,
                source_node: lead,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        )]),
    };
    let fixture = Fixture {
        lead,
        edge,
        volume: AutomationTarget::TrackVolume { track_id: lead },
        pan: AutomationTarget::TrackPan { track_id: lead },
        param: AutomationTarget::DeviceParam {
            track_id: lead,
            slot_index: 0,
            param_index: 0,
        },
        macro_target: AutomationTarget::Macro {
            track_id: lead,
            macro_index: 0,
        },
        send: AutomationTarget::SendGain {
            track_id: lead,
            edge_id: edge,
        },
    };
    doc.validate().expect("夹具本身必须合法");
    (doc, fixture)
}

/// 把泳道装进夹具文档的 Lead 轨。
fn with_lane(target: AutomationTarget, automation: AutomationLane) -> (YebanProjectV1, Fixture) {
    let (mut doc, fixture) = fixture();
    doc.tracks
        .get_mut(&fixture.lead)
        .expect("lead")
        .automation_lanes
        .insert(target, automation);
    (doc, fixture)
}

// ---------------------------------------------------------------------------
// ① 求值口径：线性 / 四种曲线形状（公式 + 容差）
// ---------------------------------------------------------------------------

/// ①a 线性插值在可精确表示的 `t` 上**逐位**等于文档公式 `va + (vb-va)·t`。
///
/// 采样区间 `[0,1000]`、端点 `0.0 → 1.0`，于是 `t = tick/1000` 在
/// `tick ∈ {0,250,500,750,1000}` 上都是二进制精确值（`0, 1/4, 1/2, 3/4, 1`）。
#[test]
fn linear_interpolation_is_bitwise_equal_to_the_documented_formula() {
    let (_, f) = fixture();
    let automation = span_lane(f.volume, CurveType::Linear);
    for (tick, expected) in [
        (0_u64, 0.0_f32),
        (250, 0.25),
        (500, 0.5),
        (750, 0.75),
        (1000, 1.0),
    ] {
        let actual = automation.value_at(tick).expect("非空泳道必有值");
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "tick={tick}: 线性插值必须逐位等于 {expected}"
        );
    }
}

/// ①b 四个曲线形状在 `t = 1/4` 与 `t = 1/2` 上逐位等于文档公式：
/// `Linear: t`、`Exponential: t²`、`Logarithmic: t·(2−t)`、`SCurve: t²·(3−2t)`。
#[test]
fn all_four_curve_shapes_match_the_documented_formulas() {
    let (_, f) = fixture();
    // (curve, t, 期望 u) —— 全部都是二进制精确值。
    let cases = [
        (CurveType::Linear, 0.25_f32, 0.25_f32),
        (CurveType::Linear, 0.5, 0.5),
        (CurveType::Exponential, 0.25, 0.0625),
        (CurveType::Exponential, 0.5, 0.25),
        (CurveType::Logarithmic, 0.25, 0.4375),
        (CurveType::Logarithmic, 0.5, 0.75),
        (CurveType::SCurve, 0.25, 0.156_25),
        (CurveType::SCurve, 0.5, 0.5),
    ];
    for (curve, t, expected) in cases {
        assert_eq!(
            curve.ease(t).to_bits(),
            expected.to_bits(),
            "{curve:?}.ease({t}) 必须逐位等于 {expected}"
        );
        // 端点 0.0 → 1.0 ⇒ 泳道求值结果就是 u 本身（证明求值确实复用了这个口径）。
        let automation = span_lane(f.volume, curve);
        let tick = u64::from((t * 1000.0) as u32);
        let actual = automation.value_at(tick).expect("非空泳道必须有值");
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{curve:?} 在 t={t} (tick={tick}) 处的求值必须等于 {expected}"
        );
    }
}

/// ①c 非二进制精确的一般情形：与文档公式对账，绝对误差容差 `1e-6`。
#[test]
fn interpolation_on_a_non_dyadic_span_matches_the_formula_within_1e_6() {
    let (_, f) = fixture();
    // 区间 [0,3]、0.0 → 1.0：t = 1/3 在 f32 下不精确。
    for curve in [
        CurveType::Linear,
        CurveType::Exponential,
        CurveType::Logarithmic,
        CurveType::SCurve,
    ] {
        let automation = lane(
            f.volume,
            vec![
                point(100, 0, 0.0, curve),
                point(101, 3, 1.0, CurveType::Linear),
            ],
        );
        for tick in 0..=3_u64 {
            let t = (tick as f32) / 3.0_f32;
            let expected = curve.ease(t);
            let actual = automation.value_at(tick).expect("非空泳道必须有值");
            assert!(
                (actual - expected).abs() <= 1e-6,
                "{curve:?} tick={tick}: 实测 {actual}, 公式 {expected}"
            );
        }
    }
}

/// ①d 形状本身的性质：两端精确、单调不减、值域 `[0,1]`、越界钳位。
#[test]
fn curve_shapes_hit_both_endpoints_and_are_monotone() {
    for curve in [
        CurveType::Linear,
        CurveType::Exponential,
        CurveType::Logarithmic,
        CurveType::SCurve,
    ] {
        assert_eq!(curve.ease(0.0), 0.0, "{curve:?}: u(0) 必须精确为 0");
        assert_eq!(curve.ease(1.0), 1.0, "{curve:?}: u(1) 必须精确为 1");
        assert_eq!(curve.ease(-3.0), 0.0, "越界一律钳到 0");
        assert_eq!(curve.ease(7.0), 1.0, "越界一律钳到 1");
        let mut previous = f32::NEG_INFINITY;
        for step in 0..=1000_u32 {
            let value = curve.ease(step as f32 / 1000.0);
            assert!(
                value >= previous - 1e-7,
                "{curve:?}: 必须单调不减 (step={step}: {value} < {previous})"
            );
            assert!((0.0..=1.0).contains(&value), "{curve:?}: u 必须落在 [0,1]");
            previous = value;
        }
    }
}

// ---------------------------------------------------------------------------
// ② 边界：空泳道 / 单点 / 首尾之外 / 命中点 / 同 tick
// ---------------------------------------------------------------------------

/// ②a 空泳道没有值（`None`，而不是 `0.0` —— `0.0` 会被下游误当成"自动化到 0"）。
#[test]
fn empty_lane_yields_no_value() {
    let (_, f) = fixture();
    let automation = lane(f.volume, vec![]);
    assert!(automation.is_implicit());
    for tick in [0_u64, 1, 960, u64::MAX] {
        assert_eq!(
            automation.value_at(tick),
            None,
            "空泳道在 tick={tick} 必须无值"
        );
    }
    let (doc_with_lane, fixture_again) = with_lane(f.volume, automation);
    assert_eq!(
        doc_with_lane.automation_value_at(&fixture_again.volume, 960),
        Ok(None),
        "工程级入口对空泳道同样返回 None"
    );
}

/// ②b 单点泳道处处保持该点的值（含 `tick = 0` 与 `tick = u64::MAX`）。
#[test]
fn single_point_lane_holds_its_value_everywhere() {
    let (_, f) = fixture();
    let automation = lane(f.volume, vec![point(100, 1920, -7.5, CurveType::SCurve)]);
    for tick in [0_u64, 1, 1919, 1920, 1921, 960_000, u64::MAX] {
        let actual = automation.value_at(tick).expect("单点泳道必须有值");
        assert_eq!(
            actual.to_bits(),
            (-7.5_f32).to_bits(),
            "单点泳道在 tick={tick} 必须保持 -7.5"
        );
    }
}

/// ②c 首点之前 / 末点之后一律**保持**（不外推），末点自身逐位精确，
/// 区间内部仍然是插值（不是阶梯）。
#[test]
fn ticks_outside_the_point_range_hold_the_boundary_value() {
    let (_, f) = fixture();
    let automation = lane(
        f.volume,
        vec![
            point(100, 960, -6.0, CurveType::Linear),
            point(101, 3840, 0.0, CurveType::SCurve),
        ],
    );
    for tick in [0_u64, 1, 959] {
        assert_eq!(
            automation.value_at(tick).map(f32::to_bits),
            Some((-6.0_f32).to_bits()),
            "首点之前 (tick={tick}) 必须保持首点值"
        );
    }
    for tick in [3840_u64, 3841, 100_000, u64::MAX] {
        assert_eq!(
            automation.value_at(tick).map(f32::to_bits),
            Some(0.0_f32.to_bits()),
            "末点之后 (tick={tick}) 必须保持末点值"
        );
    }
    let middle = automation.value_at(2400).expect("区间内必须有值");
    assert!(
        (middle - (-3.0)).abs() <= 1e-6,
        "区间内必须是插值（SCurve 在 t=0.5 处恰为中点）: {middle}"
    );
}

/// ②d 命中采样点时逐位精确返回该点的值（不受该点曲线形状影响）。
#[test]
fn exact_point_hits_return_the_point_value_bitwise() {
    let (_, f) = fixture();
    let automation = lane(
        f.volume,
        vec![
            point(100, 0, -11.5, CurveType::Exponential),
            point(101, 480, 3.25, CurveType::Logarithmic),
            point(102, 960, -0.125, CurveType::SCurve),
        ],
    );
    for (tick, expected) in [(0_u64, -11.5_f32), (480, 3.25), (960, -0.125)] {
        assert_eq!(
            automation.value_at(tick).map(f32::to_bits),
            Some(expected.to_bits()),
            "tick={tick} 命中采样点必须逐位等于 {expected}"
        );
    }
}

/// ②e 同一 tick 上多个采样点有一个**确定**的胜者：`point_id` 字典序最大者
/// （与插入顺序、`BTreeMap` 键序都无关）。
#[test]
fn same_tick_points_resolve_deterministically_by_point_id() {
    let (_, f) = fixture();
    let low = point(100, 960, -20.0, CurveType::Linear);
    let high = point(200, 960, -10.0, CurveType::Linear);
    let automation = lane(f.volume, vec![high, low]);
    assert_eq!(
        automation.value_at(960).map(f32::to_bits),
        Some((-10.0_f32).to_bits()),
        "同 tick 的胜者必须是 point_id 更大者"
    );
    assert_eq!(
        automation.value_at(0).map(f32::to_bits),
        Some((-20.0_f32).to_bits()),
        "同 tick 时首前保持取 (tick,id) 序的第一个点"
    );
}

/// ②f 采样点序列按 `(tick, point_id)` 升序，且与插入顺序无关。
#[test]
fn points_in_tick_order_is_deterministic_and_ordered() {
    let (_, f) = fixture();
    let automation = lane(
        f.volume,
        vec![
            point(300, 960, 1.0, CurveType::Linear),
            point(100, 0, 2.0, CurveType::Linear),
            point(200, 960, 3.0, CurveType::Linear),
        ],
    );
    let ordered: Vec<(u64, EntityId)> = automation
        .points_in_tick_order()
        .iter()
        .map(|p| (p.tick, p.id))
        .collect();
    assert_eq!(
        ordered,
        vec![(0, id(100)), (960, id(200)), (960, id(300))],
        "必须按 (tick, point_id) 升序"
    );
}

// ---------------------------------------------------------------------------
// ③ 新 Op 的 apply_inverse 真逆（逐字节）
// ---------------------------------------------------------------------------

/// ③a `SetAutomationLane` 的新建 + 修改两种形态都逐字节可逆。
#[test]
fn set_automation_lane_is_a_true_inverse_byte_for_byte() {
    let (mut doc, f) = fixture();
    let before = serde_json::to_string(&doc).expect("serialize");

    // 新建（old_lane == None）：写模式 + 显式取值域 ⇒ 与隐式泳道可区分。
    let mut created = lane(f.volume, vec![point(100, 0, -6.0, CurveType::Linear)]);
    created.write_mode = AutomationWriteMode::Touch;
    created.domain = Some(AutomationValueDomain::new(-60.0, 12.0).expect("有限端点"));
    let create = Op::SetAutomationLane {
        target: f.volume,
        old_lane: None,
        new_lane: created.clone(),
    };
    create.apply(&mut doc).expect("新建泳道");
    assert_ne!(
        serde_json::to_string(&doc).expect("serialize"),
        before,
        "Op 必须真的改变文档"
    );
    assert_eq!(doc.automation_lane(&f.volume), Some(&created));
    create.apply_inverse(&mut doc).expect("撤销新建泳道");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        before,
        "撤销新建后必须逐字节回到原状"
    );

    // 修改（old_lane == Some）：把读开关关掉。
    let mut replacement = created.clone();
    replacement.read_enabled = false;
    let modify = Op::SetAutomationLane {
        target: f.volume,
        old_lane: Some(created.clone()),
        new_lane: replacement.clone(),
    };
    create.apply(&mut doc).expect("先建");
    modify.apply(&mut doc).expect("改写");
    assert_eq!(doc.automation_lane(&f.volume), Some(&replacement));
    modify.apply_inverse(&mut doc).expect("撤销改写");
    assert_eq!(
        doc.automation_lane(&f.volume),
        Some(&created),
        "撤销改写必须精确还原上一条泳道"
    );
    create.apply_inverse(&mut doc).expect("撤销新建");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        before,
        "全链撤销后必须逐字节回到原状"
    );
}

/// ③b `RemoveAutomationLane` 逐字节可逆（含"空但带属性"的泳道）。
#[test]
fn remove_automation_lane_is_a_true_inverse_byte_for_byte() {
    let (mut doc, f) = fixture();
    let mut automation = lane(f.volume, vec![]);
    automation.write_mode = AutomationWriteMode::Latch;

    doc.tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(f.volume, automation.clone());
    let setup = serde_json::to_string(&doc).expect("serialize");

    let remove = Op::RemoveAutomationLane {
        target: f.volume,
        previous_lane: automation,
    };
    remove.apply(&mut doc).expect("删除泳道");
    assert_eq!(doc.automation_lane(&f.volume), None);
    assert_ne!(
        serde_json::to_string(&doc).expect("serialize"),
        setup,
        "Op 必须真的改变文档"
    );
    remove.apply_inverse(&mut doc).expect("撤销删除泳道");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        setup,
        "撤销删除必须逐字节还原（含取值域/写模式/采样点）"
    );
}

/// ③c 采样点 Op 的**自动建/自动收**精确互逆；带属性的泳道被移空后**不**被回收，
/// 因此它的属性不会在"移空最后一个点 → 撤销"这一步丢失。
#[test]
fn implicit_lane_lifecycle_is_exactly_reversible() {
    let (mut doc, f) = fixture();
    let target = AutomationTarget::TrackPan { track_id: f.lead };
    let before = serde_json::to_string(&doc).expect("serialize");

    // 隐式泳道：`SetAutomationPoint` 自动建，`RemoveAutomationPoint` 自动收。
    let sample = point(100, 480, 0.5, CurveType::Linear);
    let set = Op::SetAutomationPoint {
        target,
        point_id: sample.id,
        old_point: None,
        new_point: sample,
    };
    set.apply(&mut doc).expect("写点");
    let implicit = doc.automation_lane(&target).expect("自动建了泳道").clone();
    assert_eq!(
        implicit,
        with_point(AutomationLane::implicit(target), sample)
    );
    set.apply_inverse(&mut doc).expect("撤销写点");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        before,
        "隐式泳道建了又收，必须逐字节回到原状"
    );

    // 带属性的泳道：移空最后一个点后**保留**（否则属性会丢），撤销后逐字节还原。
    let mut attributed = AutomationLane::implicit(target);
    attributed.write_mode = AutomationWriteMode::Touch;
    let attributed_with_point = with_point(attributed.clone(), sample);
    doc.tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(target, attributed_with_point);
    let setup = serde_json::to_string(&doc).expect("serialize");

    let remove_point = Op::RemoveAutomationPoint {
        target,
        point_id: sample.id,
        previous_point: sample,
    };
    remove_point.apply(&mut doc).expect("移空最后一个点");
    assert_eq!(
        doc.automation_lane(&target),
        Some(&attributed),
        "带写模式的泳道被移空后必须保留（属性不能丢）"
    );
    remove_point.apply_inverse(&mut doc).expect("撤销移点");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        setup,
        "撤销移点必须逐字节还原"
    );
}

// ---------------------------------------------------------------------------
// ④ 序列化往返
// ---------------------------------------------------------------------------

/// ④a 完整形状的 serde 往返：`to_value → from_value` 结构相同、两次序列化逐字节相同。
#[test]
fn serde_round_trip_of_the_full_shape() {
    let (mut doc, f) = fixture();
    let mut automation = lane(
        f.param,
        vec![
            point(100, 0, 0.0, CurveType::SCurve),
            point(101, 960, 1.0, CurveType::Exponential),
        ],
    );
    automation.domain = Some(AutomationValueDomain::new(0.1, 0.9).expect("有限端点"));
    automation.write_mode = AutomationWriteMode::Write;
    doc.tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(f.param, automation);

    let value = serde_json::to_value(&doc).expect("serialize");
    let back: YebanProjectV1 = serde_json::from_value(value).expect("deserialize");
    assert_eq!(back, doc, "serde 往返必须结构相同");
    assert_eq!(back.validate(), Ok(()), "往返后的文档必须仍然合法");
    let first = serde_json::to_string(&doc).expect("serialize");
    let second = serde_json::to_string(&doc).expect("serialize");
    assert_eq!(first, second, "同一文档两次序列化必须逐字节相同");
}

/// ④b 取值域与写模式的 JSON 形状稳定：端点排序、默认值不落盘、非有限端点被拒。
#[test]
fn value_domain_and_write_mode_json_shape_is_stable() {
    // 端点顺序无关：`(12.0, -60.0)` 与 `(-60.0, 12.0)` 是**同一个**区间。
    let flipped = AutomationValueDomain::new(12.0, -60.0).expect("有限端点");
    let straight = AutomationValueDomain::new(-60.0, 12.0).expect("有限端点");
    assert_eq!(flipped, straight);
    assert_eq!(flipped.min(), -60.0);
    assert_eq!(flipped.max(), 12.0);
    assert_eq!(flipped.span(), 72.0);
    assert_eq!(
        serde_json::to_string(&flipped).expect("serialize"),
        r#"{"min":-60.0,"max":12.0}"#
    );
    // 反序列化同样把端点排序（`min > max` 因此在模型里**不可表示**）。
    let parsed: AutomationValueDomain =
        serde_json::from_str(r#"{"min":12.0,"max":-60.0}"#).expect("deserialize");
    assert_eq!(parsed, straight);
    // 非有限端点必须被拒绝（否则无法确定性序列化）。
    let error = AutomationValueDomain::new(f32::NAN, 1.0).expect_err("NaN 必须被拒");
    assert!(
        matches!(error, ModelError::NonFiniteValue { .. }),
        "{error:?}"
    );
    assert!(
        serde_json::from_str::<AutomationValueDomain>(r#"{"min":0.0,"max":null}"#).is_err(),
        "非数值端点必须被拒"
    );
    assert!(
        serde_json::from_str::<AutomationValueDomain>(r#"{"min":0.0,"max":1.0,"x":2.0}"#).is_err(),
        "未知字段必须被拒"
    );

    // 默认值全部**不落盘**：`read_enabled=true` / `write_mode=Off` / `domain=None`。
    let (_, f) = fixture();
    let default_shaped = lane(f.volume, vec![point(100, 0, -6.0, CurveType::Linear)]);
    let object = serde_json::to_value(&default_shaped)
        .expect("serialize")
        .as_object()
        .cloned()
        .expect("object");
    assert!(!object.contains_key("read_enabled"), "默认 true 不落盘");
    assert!(!object.contains_key("write_mode"), "默认 Off 不落盘");
    assert!(!object.contains_key("domain"), "None 不落盘");
    // 非默认值必须落盘。
    let mut explicit = default_shaped.clone();
    explicit.read_enabled = false;
    explicit.write_mode = AutomationWriteMode::Latch;
    explicit.domain = Some(straight);
    let object = serde_json::to_value(&explicit)
        .expect("serialize")
        .as_object()
        .cloned()
        .expect("object");
    assert_eq!(object.get("read_enabled"), Some(&serde_json::json!(false)));
    assert_eq!(object.get("write_mode"), Some(&serde_json::json!("Latch")));
    assert_eq!(
        object.get("domain"),
        Some(&serde_json::json!({ "min": -60.0, "max": 12.0 }))
    );
}

// ---------------------------------------------------------------------------
// ⑤ 旧工程兼容
// ---------------------------------------------------------------------------

/// ⑤a 没有新字段的旧 JSON 能读，且新字段取到正确默认值；
/// 再导出时**逐字节不变**（新字段在默认值上完全隐身）。
#[test]
fn legacy_lane_json_reads_with_defaults_and_reserializes_byte_identically() {
    let legacy = serde_json::json!({
        "target": { "TrackVolume": { "track_id": id(2).to_string() } },
        "points": {
            id(100).to_string(): { "id": id(100).to_string(), "tick": 0, "value": -6.0, "curve": "Linear" },
            id(101).to_string(): { "id": id(101).to_string(), "tick": 3840, "value": 0.0, "curve": "SCurve" }
        }
    });
    let parsed: AutomationLane = serde_json::from_value(legacy.clone()).expect("旧 JSON 必须可读");
    assert!(parsed.read_enabled, "缺 read_enabled 必须默认 true");
    assert_eq!(parsed.write_mode, AutomationWriteMode::Off);
    assert_eq!(parsed.domain, None);
    assert_eq!(parsed.points.len(), 2);
    assert_eq!(
        serde_json::to_value(&parsed).expect("serialize"),
        legacy,
        "旧 JSON 再导出必须逐字节不变（默认值不落盘）"
    );
    // 旧泳道的有效取值域与单位仍然可判定（从目标派生）。
    assert_eq!(parsed.effective_domain(), parsed.target.nominal_domain());
    assert_eq!(parsed.unit(), AutomationUnit::Decibels);
    // 求值可用。
    assert_eq!(
        parsed.value_at(0).map(f32::to_bits),
        Some((-6.0_f32).to_bits())
    );
    assert_eq!(
        parsed.value_at(3840).map(f32::to_bits),
        Some(0.0_f32.to_bits())
    );
}

/// ⑤b 整个**旧工程文档**（`tracks[*].automation_lanes` 里的泳道没有新字段）能读、
/// 能校验、能求值，且再导出后旧泳道那一段字节不变。
#[test]
fn legacy_project_document_without_the_new_fields_is_still_readable() {
    let (doc, f) = fixture();
    let mut json = serde_json::to_value(&doc).expect("serialize");
    // 手工塞一条"旧形状"的泳道（只有 target + points）进去。
    let legacy_lane = serde_json::json!({
        "target": { "TrackVolume": { "track_id": f.lead.to_string() } },
        "points": {
            id(100).to_string(): { "id": id(100).to_string(), "tick": 0, "value": -9.0, "curve": "Linear" },
            id(101).to_string(): { "id": id(101).to_string(), "tick": 960, "value": -3.0, "curve": "Linear" }
        }
    });
    json["tracks"][f.lead.to_string()]["automation_lanes"] =
        serde_json::Value::Array(vec![legacy_lane.clone()]);
    let reparsed: YebanProjectV1 = serde_json::from_value(json).expect("旧工程必须可读");
    assert_eq!(reparsed.validate(), Ok(()));
    let value = reparsed
        .automation_value_at(&f.volume, 480)
        .expect("目标存在")
        .expect("非空泳道有值");
    assert!(
        (value - (-6.0)).abs() <= 1e-6,
        "线性中点必须是 -6.0，实测 {value}"
    );

    // 再导出后，该泳道那一段必须**逐字节等于**旧形状。
    let exported = serde_json::to_value(&reparsed).expect("serialize");
    assert_eq!(
        exported["tracks"][f.lead.to_string()]["automation_lanes"][0],
        legacy_lane,
        "旧泳道再导出必须逐字节不变"
    );
}

// ---------------------------------------------------------------------------
// ⑥ 确定性
// ---------------------------------------------------------------------------

/// ⑥a 同一泳道同一 tick 反复求值逐位相同（每个 tick 1000 次）。
#[test]
fn evaluation_is_bitwise_deterministic_across_repeated_calls() {
    let (_, f) = fixture();
    let points = || {
        vec![
            point(100, 0, -18.5, CurveType::Linear),
            point(101, 960, 5.5, CurveType::Exponential),
            point(102, 1920, -3.25, CurveType::Logarithmic),
            point(103, 3840, 0.125, CurveType::SCurve),
        ]
    };
    let automation = lane(f.volume, points());
    for tick in [
        0_u64, 1, 480, 959, 960, 961, 1440, 1920, 2880, 3839, 3840, 4000,
    ] {
        let reference = automation.value_at(tick).map(f32::to_bits);
        for _ in 0..1000 {
            assert_eq!(automation.value_at(tick).map(f32::to_bits), reference);
        }
    }
    // 两份独立构造的等价泳道也必须给出同一位模式。
    let twin = lane(f.volume, points());
    for tick in [0_u64, 500, 960, 2000, 3840] {
        assert_eq!(
            automation.value_at(tick).map(f32::to_bits),
            twin.value_at(tick).map(f32::to_bits)
        );
    }
}

/// ⑥b 求值与序列化都不依赖 `BTreeMap` 的插入序。
#[test]
fn evaluation_and_serialization_are_independent_of_insertion_order() {
    let (_, f) = fixture();
    let forward = lane(
        f.volume,
        vec![
            point(100, 0, 0.0, CurveType::Linear),
            point(101, 960, 1.0, CurveType::SCurve),
            point(102, 1920, 0.25, CurveType::Exponential),
        ],
    );
    let backward = lane(
        f.volume,
        vec![
            point(102, 1920, 0.25, CurveType::Exponential),
            point(101, 960, 1.0, CurveType::SCurve),
            point(100, 0, 0.0, CurveType::Linear),
        ],
    );
    assert_eq!(forward, backward);
    assert_eq!(
        serde_json::to_string(&forward).expect("serialize"),
        serde_json::to_string(&backward).expect("serialize")
    );
    for tick in [0_u64, 480, 960, 1200, 1920, 5000] {
        assert_eq!(forward.value_at(tick), backward.value_at(tick));
    }
}

// ---------------------------------------------------------------------------
// ⑦ 与既有 AutomationTarget 对账
// ---------------------------------------------------------------------------

/// ⑦a 单位的派生表：五个目标变体的固有单位固定。
#[test]
fn nominal_units_are_derived_from_the_target() {
    let (_, f) = fixture();
    assert_eq!(f.volume.nominal_unit(), AutomationUnit::Decibels);
    assert_eq!(f.send.nominal_unit(), AutomationUnit::Decibels);
    assert_eq!(f.pan.nominal_unit(), AutomationUnit::Bipolar);
    assert_eq!(f.macro_target.nominal_unit(), AutomationUnit::Normalized);
    assert_eq!(f.param.nominal_unit(), AutomationUnit::Native);
    assert_eq!(AutomationUnit::Decibels.symbol(), "dB");
    assert_eq!(AutomationUnit::Native.symbol(), "");
}

/// ⑦b 取值域的派生表：已知目标有固有区间，设备参数诚实地"不可知"（`None`）。
#[test]
fn nominal_domains_are_derived_from_the_target() {
    let (_, f) = fixture();
    let volume = f.volume.nominal_domain().expect("音量有固有取值域");
    assert_eq!((volume.min(), volume.max()), (-60.0, 12.0));
    let pan = f.pan.nominal_domain().expect("声相有固有取值域");
    assert_eq!((pan.min(), pan.max()), (-1.0, 1.0));
    let macro_domain = f.macro_target.nominal_domain().expect("宏有固有取值域");
    assert_eq!((macro_domain.min(), macro_domain.max()), (0.0, 1.0));
    assert_eq!(
        f.param.nominal_domain(),
        None,
        "设备参数的取值域在模型里不可知, 必须返回 None 而不是编一个 0..=1"
    );
    // 泳道的有效取值域 = 显式覆盖优先，否则目标的固有取值域；单位永远由目标派生。
    let mut automation = lane(f.volume, vec![]);
    assert_eq!(automation.effective_domain(), Some(volume));
    assert_eq!(automation.unit(), AutomationUnit::Decibels);
    automation.domain = Some(AutomationValueDomain::new(-24.0, 6.0).expect("有限端点"));
    assert_eq!(
        automation.effective_domain().map(|d| (d.min(), d.max())),
        Some((-24.0, 6.0))
    );
    assert_eq!(automation.unit(), AutomationUnit::Decibels);
}

/// ⑦c 目标不存在时给出**具体**错误（不是笼统的"找不到"），且工程级入口先对账后求值。
#[test]
fn automation_value_at_reconciles_the_target_and_reports_specific_errors() {
    let (doc, f) = fixture();
    let missing_track = AutomationTarget::TrackVolume { track_id: id(999) };
    assert_eq!(
        doc.automation_value_at(&missing_track, 0),
        Err(ModelError::TrackNotFound { id: id(999) })
    );
    let missing_edge = AutomationTarget::SendGain {
        track_id: f.lead,
        edge_id: id(998),
    };
    assert_eq!(
        doc.automation_value_at(&missing_edge, 0),
        Err(ModelError::RoutingEdgeNotFound { id: id(998) })
    );
    let missing_slot = AutomationTarget::DeviceParam {
        track_id: f.lead,
        slot_index: 7,
        param_index: 0,
    };
    assert_eq!(
        doc.automation_value_at(&missing_slot, 0),
        Err(ModelError::DeviceSlotOutOfRange { index: 7, len: 1 })
    );
    let missing_param = AutomationTarget::DeviceParam {
        track_id: f.lead,
        slot_index: 0,
        param_index: 9,
    };
    assert_eq!(
        doc.automation_value_at(&missing_param, 0),
        Err(ModelError::ParamIndexOutOfRange { index: 9, len: 2 })
    );
    let missing_macro = AutomationTarget::Macro {
        track_id: f.lead,
        macro_index: 3,
    };
    assert_eq!(
        doc.automation_value_at(&missing_macro, 0),
        Err(ModelError::MacroIndexOutOfRange { index: 3, len: 1 })
    );
    // 存在的目标（没有泳道）是 `Ok(None)`，不是错误。
    assert_eq!(doc.automation_value_at(&f.volume, 0), Ok(None));
    assert_eq!(f.volume.validate_against(&doc), Ok(()));
}

/// ⑦d 静态值入口：音量/声相/设备参数/宏直接可取；发送增益的 `None` 是"单位增益 = 0 dB"。
#[test]
fn static_value_covers_every_target_including_send_gain() {
    let (mut doc, f) = fixture();
    assert_eq!(f.volume.static_value(&doc), Ok(-3.0));
    assert_eq!(f.pan.static_value(&doc), Ok(-0.25));
    assert_eq!(f.param.static_value(&doc), Ok(1200.0));
    assert_eq!(f.macro_target.static_value(&doc), Ok(0.5));
    // `gain_db == None` ⇒ 单位增益 0.0 dB（`SetRoutingGain` 的语义）。
    assert_eq!(f.send.static_value(&doc), Ok(0.0));
    doc.routing_graph
        .edges
        .get_mut(&f.edge)
        .expect("edge")
        .gain_db = Some(-4.5);
    assert_eq!(f.send.static_value(&doc), Ok(-4.5));
    let missing_edge = AutomationTarget::SendGain {
        track_id: f.lead,
        edge_id: id(997),
    };
    assert_eq!(
        missing_edge.static_value(&doc),
        Err(ModelError::RoutingEdgeNotFound { id: id(997) })
    );
}

/// ⑦e `read_enabled == false` 的泳道不参与播放：工程级入口返回 `Ok(None)`，
/// 下游应当退回静态值 —— 但泳道本身与采样点**仍然在文档里**（可再次打开）。
#[test]
fn read_disabled_lane_yields_no_value_but_keeps_its_points() {
    let (_, f) = fixture();
    let mut automation = lane(f.pan, vec![point(100, 0, 0.75, CurveType::Linear)]);
    automation.read_enabled = false;
    let (doc, fixture_again) = with_lane(f.pan, automation.clone());
    assert_eq!(
        doc.automation_value_at(&fixture_again.pan, 0),
        Ok(None),
        "关掉的泳道不得参与求值"
    );
    assert_eq!(
        doc.automation_lane(&fixture_again.pan)
            .map(|stored| stored.points.len()),
        Some(1),
        "关掉泳道不等于丢掉采样点"
    );
    assert_eq!(
        fixture_again.pan.static_value(&doc),
        Ok(-0.25),
        "退回静态值"
    );
    // 纯计算入口不认识"读开关"（它只认泳道本身），这是刻意的分层：
    // 读开关属于**工程级**求值入口（渲染/引擎应当调用的那一个）。
    assert_eq!(automation.value_at(0), Some(0.75));
}

// ---------------------------------------------------------------------------
// ⑧ 两个新 Op 的拒绝路径与批量可逆
// ---------------------------------------------------------------------------

/// ⑧a 载荷与文档不一致时必须拒绝，且**不改变文档**。
#[test]
fn lane_ops_reject_inconsistent_payloads_without_mutating() {
    let (mut doc, f) = fixture();
    let existing = lane(f.volume, vec![point(100, 0, -6.0, CurveType::Linear)]);
    doc.tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(f.volume, existing.clone());
    let snapshot = serde_json::to_string(&doc).expect("serialize");

    // ① 内嵌 target 与键不一致。
    let mut wrong_target = existing.clone();
    wrong_target.target = f.pan;
    let error = Op::SetAutomationLane {
        target: f.volume,
        old_lane: Some(existing.clone()),
        new_lane: wrong_target,
    }
    .apply(&mut doc)
    .expect_err("target 不一致必须拒绝");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ② 显式写入"与隐式泳道不可区分"的泳道。
    let error = Op::SetAutomationLane {
        target: f.pan,
        old_lane: None,
        new_lane: AutomationLane::implicit(f.pan),
    }
    .apply(&mut doc)
    .expect_err("隐式形状的泳道不得被显式创建");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ③ `old_lane` 与文档不符。
    let error = Op::SetAutomationLane {
        target: f.volume,
        old_lane: None,
        new_lane: existing.clone(),
    }
    .apply(&mut doc)
    .expect_err("old_lane 不符必须拒绝");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ④ 删除不存在的泳道。
    let error = Op::RemoveAutomationLane {
        target: f.pan,
        previous_lane: existing.clone(),
    }
    .apply(&mut doc)
    .expect_err("泳道不存在必须拒绝");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ⑤ 删除时载荷不符。
    let mut other = existing.clone();
    other.read_enabled = false;
    let error = Op::RemoveAutomationLane {
        target: f.volume,
        previous_lane: other,
    }
    .apply(&mut doc)
    .expect_err("previous_lane 不符必须拒绝");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ⑥ 删除"隐式形状"的载荷（该形状由采样点 Op 拥有，不由本变体负责）。
    let error = Op::RemoveAutomationLane {
        target: f.pan,
        previous_lane: AutomationLane::implicit(f.pan),
    }
    .apply(&mut doc)
    .expect_err("隐式形状不得由 RemoveAutomationLane 负责");
    assert!(
        matches!(error, ModelError::OpStateMismatch { .. }),
        "{error:?}"
    );

    // ⑦ 采样点取值非有限。
    let error = Op::SetAutomationLane {
        target: f.pan,
        old_lane: None,
        new_lane: lane(f.pan, vec![point(100, 0, f32::INFINITY, CurveType::Linear)]),
    }
    .apply(&mut doc)
    .expect_err("非有限取值必须拒绝");
    assert!(
        matches!(error, ModelError::NonFiniteValue { .. }),
        "{error:?}"
    );

    // ⑧ 目标音轨不存在。
    let ghost = AutomationTarget::TrackVolume { track_id: id(999) };
    let error = Op::SetAutomationLane {
        target: ghost,
        old_lane: None,
        new_lane: lane(ghost, vec![point(100, 0, 0.0, CurveType::Linear)]),
    }
    .apply(&mut doc)
    .expect_err("音轨不存在必须拒绝");
    assert!(
        matches!(error, ModelError::TrackNotFound { .. }),
        "{error:?}"
    );

    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        snapshot,
        "每一次拒绝都不得改变文档"
    );
    assert_eq!(doc.automation_lane(&f.volume), Some(&existing));
}

/// ⑧b 两个新 Op 在**批量**里也逐字节可逆（含"建泳道 → 加点 → 移点"的交叉）。
#[test]
fn lane_ops_inside_a_batch_are_reversible() {
    let (mut doc, f) = fixture();
    let before = serde_json::to_string(&doc).expect("serialize");
    let mut created = lane(f.volume, vec![point(100, 0, -6.0, CurveType::Linear)]);
    created.write_mode = AutomationWriteMode::Touch;
    let batch = Op::Batch {
        ops: vec![
            Op::SetAutomationLane {
                target: f.volume,
                old_lane: None,
                new_lane: created.clone(),
            },
            Op::SetAutomationPoint {
                target: f.volume,
                point_id: id(101),
                old_point: None,
                new_point: point(101, 960, -3.0, CurveType::SCurve),
            },
            Op::RemoveAutomationPoint {
                target: f.volume,
                point_id: id(100),
                previous_point: point(100, 0, -6.0, CurveType::Linear),
            },
        ],
        description: "自动化录入".to_owned(),
    };
    batch.apply(&mut doc).expect("批量应用");
    assert_eq!(
        doc.automation_lane(&f.volume)
            .map(|automation| automation.points.len()),
        Some(1),
        "移点后应只剩一个点"
    );
    batch.apply_inverse(&mut doc).expect("批量撤销");
    assert_eq!(
        serde_json::to_string(&doc).expect("serialize"),
        before,
        "批量撤销必须逐字节回到原状"
    );
}

/// ⑧c 新泳道字段在工程级校验里必须合法；键与内嵌 target 不一致仍然被拒（既有判据不削弱）。
#[test]
fn documents_with_the_new_lane_fields_stay_valid() {
    let (mut doc, f) = fixture();
    let mut automation = lane(
        f.volume,
        vec![
            point(100, 0, -60.0, CurveType::Linear),
            point(101, 3840, 12.0, CurveType::SCurve),
        ],
    );
    automation.write_mode = AutomationWriteMode::Touch;
    automation.domain = Some(AutomationValueDomain::new(-60.0, 12.0).expect("有限端点"));
    doc.tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(f.volume, automation);
    assert_eq!(doc.validate(), Ok(()));

    let mut broken = doc.clone();
    broken
        .tracks
        .get_mut(&f.lead)
        .expect("lead")
        .automation_lanes
        .insert(f.pan, lane(f.volume, vec![]));
    assert!(matches!(
        broken.validate(),
        Err(ModelError::AutomationLaneTargetMismatch { .. })
    ));
}

/// ⑧d `name()` 与 JSON 标签一致（新变体的契约名字必须可直接用于序列化判据）。
#[test]
fn new_op_names_match_their_json_tags() {
    let (_, f) = fixture();
    let ops = vec![
        Op::SetAutomationLane {
            target: f.pan,
            old_lane: None,
            new_lane: with_point(
                AutomationLane::implicit(f.pan),
                point(100, 0, 0.5, CurveType::Linear),
            ),
        },
        Op::RemoveAutomationLane {
            target: f.pan,
            previous_lane: with_point(
                AutomationLane::implicit(f.pan),
                point(100, 0, 0.5, CurveType::Linear),
            ),
        },
    ];
    for op in ops {
        let expected = match &op {
            Op::SetAutomationLane { .. } => "SetAutomationLane",
            Op::RemoveAutomationLane { .. } => "RemoveAutomationLane",
            other => panic!("只应出现两个新变体: {other:?}"),
        };
        assert_eq!(op.name(), expected);
        let value = serde_json::to_value(&op).expect("serialize");
        let object = value.as_object().expect("externally tagged object");
        assert_eq!(object.len(), 1, "必须是单键外部标签");
        assert!(object.contains_key(expected), "JSON 键必须与 name() 同名");
        let back: Op = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, op, "必须能往返");
    }
}
