//! `yeban_edit_automation` 的实现 —— 自动化泳道的**读**与**写一个点**
//! （`ADR-0001` **D46** 的第 1 类能力）。
//!
//! ## 读：走**唯一求值入口**，没有第二份插值
//!
//! 本模块对"某个 tick 该用哪个值"**只**调用
//! [`YebanProjectV1::automation_value_at`]（模型层唯一入口）。它自己**不**排序采样点、
//! **不**调 `CurveType::ease`、**不**做任何线性插值 —— 这一条由
//! [`super::automation_audit`] 的机械审计（扫描全部生产源码）与
//! `tests/extension_tools.rs::no_second_automation_evaluation_in_production_sources`
//! 共同钉住，而不是靠这段注释。
//!
//! 求值有三态，本模块**原样**转达（不折叠成 `null`）：
//!
//! | 情形 | `automationValue` | `source` | 说明 |
//! | :--- | :--- | :--- | :--- |
//! | 泳道有值且 `readEnabled` | `Some(v)` | `automation` | `effectiveValue = v` |
//! | 无泳道 / 空泳道 / `readEnabled == false` | `null` | `static` | `effectiveValue = ` [`AutomationTarget::static_value`] |
//!
//! 第三条**不是**发明：模型文档要求下游在"无自动化值"时退回静态值，
//! 而静态值同样只有**一个**来源（`AutomationTarget::static_value`），
//! 因此本模块不去读 `track.volume_db` / `device.params[i].value`。
//!
//! ## 写：一个点 = 一条 `Op::SetAutomationPoint` = 一步可撤销
//!
//! - 点身份缺省由 `(目标, tick)` **确定性派生**（[`super::extension_pure::point_label`]）
//!   ⇒ 同一 tick 重复写入是**更新同一个点**，不是堆积；
//! - `old_point` 从**当前文档**读（`None` = 该点原先不存在），因此撤销是模型自己的
//!   [`Op::invert`]，本层**不写**逆操作（同 `domain/mod.rs` 的"逆操作的唯一来源"）；
//! - 写入前在**克隆体**上模拟整批 + `validate()`，失败则原工程一位不变。
//!
//! ## 为什么 `digestAfter` 在 `apply` 里被**断言**
//!
//! [`AutomationEdit::digest_after`] 是只读规划在克隆体上算出来的"施加后的内容摘要"。
//! [`apply`] 提交之后**重新**算一次真实摘要并要求两者相等 —— 不等就是
//! `CONFLICT`。这条断言让"预览说的"与"真做的"不可能漂移（判据
//! `dry_run_preview_equals_the_real_write` 在外部再证明一遍）。
//!
//! ## 操作来源（`OpOrigin`）的**诚实缺口**
//!
//! 模型 `OpOrigin` 的七个变体里**没有**"AI 直接编辑"这一档（`McpProposal` 要求一个
//! 真实存在的提案身份，而本工具不创建提案）。本模块借用最接近的一档
//! [`OpOrigin::AutomationRecord`]（"自动化写入的落盘动作"，不是实时播放），
//! 并在响应的 `origin` 里如实写出借用的事实 + 未接线清单。
//! **作者**字段仍是 `yeban-mcp`（`UndoState.author`），因此不存在"伪装成用户操作"。

use serde_json::{Map, Value};

use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveType, EntityId, Op, OpOrigin,
    YebanProjectV1,
};

use super::error::{self, Fault};
use super::extension_pure::{self, LaneKind};
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// 一次调用最多读取多少个 tick 的自动化值。
///
/// 上限是**上下文护栏**（与 `yeban_query_project` 的分页同一个理由：一次把几万个
/// 读数塞回 Agent 的上下文里，等于让 Agent 看不见自己的任务）。超过即
/// `INVALID_PARAMETER_RANGE`，绝不静默截断。
pub const MAX_READ_TICKS: usize = 256;

/// 一个 tick 上的**三态读数**（自动化值 / 静态值 / 实际生效值）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationReading {
    /// 查询的 tick。
    pub tick: u64,
    /// 唯一求值入口的返回值；`None` = "本泳道没有可用自动化值"。
    pub automation_value: Option<f32>,
    /// 目标的静态值（无自动化时的回退值）。
    pub static_value: f32,
    /// 实际生效值（自动化优先，否则静态）。
    pub effective_value: f32,
    /// `"automation"` 或 `"static"`。
    pub source: &'static str,
    /// 泳道的读开关（没有泳道时 `false` —— "没有值可读"）。
    pub read_enabled: bool,
}

impl AutomationReading {
    /// JSON 形态。
    #[must_use]
    pub fn to_value(self) -> Value {
        let mut map = Map::new();
        map.insert("tick".to_owned(), Value::from(self.tick));
        map.insert(
            "automationValue".to_owned(),
            self.automation_value.map_or(Value::Null, Value::from),
        );
        map.insert("staticValue".to_owned(), Value::from(self.static_value));
        map.insert(
            "effectiveValue".to_owned(),
            Value::from(self.effective_value),
        );
        map.insert("source".to_owned(), Value::from(self.source));
        map.insert("readEnabled".to_owned(), Value::from(self.read_enabled));
        Value::Object(map)
    }

    /// 按**唯一求值入口**读一个 tick。
    ///
    /// # Errors
    ///
    /// 目标在文档里不存在（`automation_value_at` 的目标对账失败）—— 返回具体模型错误。
    fn read(project: &YebanProjectV1, target: &AutomationTarget, tick: u64) -> Result<Self, Fault> {
        // 唯一求值入口。⚠ 不要在这里"顺手"改成 `lane.value_at`：
        // 那会绕开 `read_enabled` 开关与目标对账（审计模块会红）。
        let automation_value = project
            .automation_value_at(target, tick)
            .map_err(|error| error::from_model("自动化求值", &error))?;
        let static_value = target
            .static_value(project)
            .map_err(|error| error::from_model("静态值读取", &error))?;
        let read_enabled = project
            .automation_lane(target)
            .is_some_and(|lane| lane.read_enabled);
        let (effective_value, source) = match automation_value {
            Some(value) => (value, "automation"),
            None => (static_value, "static"),
        };
        Ok(Self {
            tick,
            automation_value,
            static_value,
            effective_value,
            source,
            read_enabled,
        })
    }
}

/// 一次**已经校验过**的自动化点写入。
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationWrite {
    /// 点身份（显式给定或由 `(目标, tick)` 派生）。
    pub point_id: EntityId,
    /// 时间位置。
    pub tick: u64,
    /// 目标值。
    pub value: f32,
    /// 到下一个点的曲线形状。
    pub curve: CurveType,
    /// 该点原先是否已经存在（`false` = 新建）。
    pub existed: bool,
    /// 原先的值（不存在时 `None`）—— 撤销载荷的直接镜像，供调用方核对。
    pub old_value: Option<f32>,
    /// 将要提交的领域操作（**唯一**的变更载体）。
    pub op: Op,
}

/// 一次自动化编辑的**完整只读规划**（`dryRun` 与真做共用同一份数据）。
#[derive(Clone, Debug)]
pub struct AutomationEdit {
    /// 目标音轨。
    pub track_id: EntityId,
    /// 目标（模型类型；唯一身份）。
    pub target: AutomationTarget,
    /// 确定性目标标签（点身份派生的输入）。
    pub target_label: String,
    /// 目标变体。
    pub lane_kind: LaneKind,
    /// 泳道快照（点按 `(tick, id)` 升序 —— 模型自己的确定性顺序）。
    pub points: Vec<AutomationPoint>,
    /// 泳道是否存在（`points` 为空且没有泳道 ⇒ `false`）。
    pub lane_exists: bool,
    /// 读开关。
    pub read_enabled: bool,
    /// 写模式（模型 `AutomationWriteMode` 的名字；没有泳道时为 `null`）。
    pub write_mode: Option<String>,
    /// 单位名（模型 `AutomationUnit` 的名字，由目标派生）。
    pub unit: String,
    /// 有效取值域 `(min, max)`（泳道显式覆盖优先，否则目标固有值域）。
    pub domain: Option<(f32, f32)>,
    /// 请求读取的 tick（保持调用方顺序）。
    pub read_ticks: Vec<u64>,
    /// 写入前的读数。
    pub values_before: Vec<AutomationReading>,
    /// 将要写入的点（`None` = 这是一次只读调用）。
    pub write: Option<AutomationWrite>,
    /// 写入后的读数（只读规划在克隆体上算出来的**预测**）。
    pub values_after: Option<Vec<AutomationReading>>,
    /// 写入前的工程内容摘要。
    pub digest_before: String,
    /// 写入后的工程内容摘要（预测；只读调用为 `None`）。
    pub digest_after: Option<String>,
}

impl AutomationEdit {
    /// 是否**只读**（没有 `point` 实参 ⇒ 一位都不改）。
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.write.is_none()
    }

    /// 将要施加的 op 清单（0 或 1 条）。
    #[must_use]
    pub fn ops(&self) -> Vec<Op> {
        self.write
            .as_ref()
            .map(|write| vec![write.op.clone()])
            .unwrap_or_default()
    }

    /// 响应的 `data`（`dryRun` 预览与真做**共用这一个函数**，因此不可能漂移）。
    ///
    /// # Errors
    ///
    /// 摘要序列化失败（容器写出失败）→ `IO_ERROR`。
    pub fn data(&self) -> Result<Value, Fault> {
        let mut lane = Map::new();
        lane.insert("exists".to_owned(), Value::from(self.lane_exists));
        lane.insert("readEnabled".to_owned(), Value::from(self.read_enabled));
        lane.insert(
            "writeMode".to_owned(),
            self.write_mode.clone().map_or(Value::Null, Value::from),
        );
        lane.insert("unit".to_owned(), Value::from(self.unit.clone()));
        lane.insert(
            "domain".to_owned(),
            self.domain.map_or(
                Value::Null,
                |(min, max)| serde_json::json!({ "min": min, "max": max }),
            ),
        );
        lane.insert("pointCount".to_owned(), Value::from(self.points.len()));
        lane.insert(
            "points".to_owned(),
            Value::Array(
                self.points
                    .iter()
                    .map(|point| {
                        serde_json::json!({
                            "id": point.id.to_canonical_string(),
                            "tick": point.tick,
                            "value": point.value,
                            "curve": curve_name(point.curve),
                        })
                    })
                    .collect(),
            ),
        );

        let written = self.write.as_ref().map(|write| {
            serde_json::json!({
                "pointId": write.point_id.to_canonical_string(),
                "tick": write.tick,
                "value": write.value,
                "curve": curve_name(write.curve),
                "existed": write.existed,
                "oldValue": write.old_value.map_or(Value::Null, Value::from),
                "op": write.op.name(),
            })
        });

        Ok(serde_json::json!({
            "trackId": self.track_id.to_canonical_string(),
            // `AutomationTarget` 是 `Copy` ⇒ 按值传（clippy `needless_borrows_for_generic_args`）。
            "target": serde_json::to_value(self.target).unwrap_or(Value::Null),
            "targetLabel": self.target_label.clone(),
            "laneKind": self.lane_kind.as_str(),
            "readOnly": self.is_read_only(),
            "lane": Value::Object(lane),
            "read": {
                "ticks": self.read_ticks.clone(),
                "values": self
                    .values_before
                    .iter()
                    .map(|reading| reading.to_value())
                    .collect::<Vec<_>>(),
                "entry": extension_pure::AUTOMATION_ENTRY,
            },
            "written": written.map_or(Value::Null, Value::from),
            "valuesAfter": self.values_after.as_ref().map_or(Value::Null, |values| {
                Value::Array(values.iter().map(|reading| reading.to_value()).collect())
            }),
            "projectDigestBefore": self.digest_before.clone(),
            "projectDigestAfter": self.digest_after.clone().map_or(Value::Null, Value::from),
            "origin": {
                "kind": "AutomationRecord",
                "author": super::AGENT_NAME,
                "note": "模型 OpOrigin 没有 `McpEdit` 变体 (needs-1); 借用最接近的 AutomationRecord",
            },
        }))
    }
}

/// 模型 `CurveType` → 规范名（**只有一份**词汇表：名字就是 `project.json` 里的写法）。
///
/// 名字表在 [`extension_pure::CURVE_NAMES`]；判据
/// `every_name_table_agrees_with_the_model_enum` 断言这里是它的**双射**
/// （四个变体一个不少、一个不多）。
#[must_use]
pub fn curve_name(curve: CurveType) -> &'static str {
    match curve {
        CurveType::Linear => "Linear",
        CurveType::Exponential => "Exponential",
        CurveType::Logarithmic => "Logarithmic",
        CurveType::SCurve => "SCurve",
    }
}

/// `curve` 实参 → 模型 `CurveType`（只接受规范名，别名一律拒绝）。
///
/// # Errors
///
/// 未知名字 → `INVALID_PARAMETER_RANGE`。
fn parse_curve(raw: Option<&Value>, context: &str) -> Result<CurveType, Fault> {
    let Some(value) = raw else {
        return Ok(CurveType::Linear);
    };
    let Some(text) = value.as_str() else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{context}.curve` 必须是字符串"),
        ));
    };
    let canonical = extension_pure::canonical_curve(text).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知曲线形状 `{text}`"),
            serde_json::json!({
                "field": format!("{context}.curve"),
                "allowed": extension_pure::CURVE_NAMES,
            }),
        )
    })?;
    // 纯层已经确认它是四个规范名之一; 这里按同一份名字表构造模型值,
    // 因此"能过校验却构造不出模型值"在类型上不可能出现。
    Ok(match canonical {
        "Linear" => CurveType::Linear,
        "Exponential" => CurveType::Exponential,
        "Logarithmic" => CurveType::Logarithmic,
        "SCurve" => CurveType::SCurve,
        other => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("纯逻辑层的曲线名表与模型不同步: `{other}`"),
                serde_json::json!({ "allowed": extension_pure::CURVE_NAMES }),
            ));
        }
    })
}

/// 由实参构造自动化目标（**唯一**的目标解析入口）。
///
/// # Errors
///
/// `lane` 未知 / 缺少该变体必需的附加实参 → `INVALID_PARAMETER_RANGE`。
pub fn parse_target(
    call_track: EntityId,
    arguments: &Map<String, Value>,
) -> Result<(AutomationTarget, LaneKind, String), Fault> {
    let lane_text = arguments
        .get("lane")
        .and_then(Value::as_str)
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "`lane` 必须是字符串"))?;
    let kind = LaneKind::parse(lane_text).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知自动化目标 `{lane_text}`"),
            serde_json::json!({
                "field": "lane",
                "allowed": extension_pure::LANE_NAMES,
                "note": "只接受 project.json 里的规范变体名 (不接受 trackVolume 这类别名)",
            }),
        )
    })?;
    let edge = if kind.needs_edge_id() {
        let text = arguments
            .get("edgeId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`SendGain` 目标必须给定 `edgeId` (路由边身份)",
                )
            })?;
        Some(parse_entity("edgeId", text)?)
    } else {
        None
    };
    let slot = if kind.needs_device_slot() {
        parse_index(arguments, "slotIndex")?
    } else {
        0
    };
    let param = if kind.needs_device_slot() {
        parse_index(arguments, "paramIndex")?
    } else {
        0
    };
    let macro_index = if kind.needs_macro_index() {
        parse_index(arguments, "macroIndex")?
    } else {
        0
    };
    let slot = to_usize("slotIndex", slot)?;
    let param = to_usize("paramIndex", param)?;
    let macro_index = to_usize("macroIndex", macro_index)?;

    let target = match kind {
        LaneKind::TrackVolume => AutomationTarget::TrackVolume {
            track_id: call_track,
        },
        LaneKind::TrackPan => AutomationTarget::TrackPan {
            track_id: call_track,
        },
        LaneKind::SendGain => AutomationTarget::SendGain {
            track_id: call_track,
            edge_id: edge
                .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "缺少 `edgeId`"))?,
        },
        LaneKind::DeviceParam => AutomationTarget::DeviceParam {
            track_id: call_track,
            slot_index: slot,
            param_index: param,
        },
        LaneKind::Macro => AutomationTarget::Macro {
            track_id: call_track,
            macro_index,
        },
    };
    let label = extension_pure::target_label(
        kind,
        &call_track.to_canonical_string(),
        edge.map(|edge| edge.to_canonical_string()).as_deref(),
        u64::from(u32::try_from(slot).unwrap_or(u32::MAX)),
        u64::from(u32::try_from(param).unwrap_or(u32::MAX)),
        u64::from(u32::try_from(macro_index).unwrap_or(u32::MAX)),
    );
    Ok((target, kind, label))
}

/// 读一个 `EntityId` 实参。
fn parse_entity(field: &str, text: &str) -> Result<EntityId, Fault> {
    use std::str::FromStr as _;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

/// 读一个非负整数实参（缺省 0）。
fn parse_index(arguments: &Map<String, Value>, field: &str) -> Result<u64, Fault> {
    match arguments.get(field) {
        None => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`{field}` 必须是非负整数, 实际收到 {value}"),
            )
        }),
    }
}

/// `u64` → `usize`（宽度不足的平台上明确报错，不截断）。
fn to_usize(field: &str, value: u64) -> Result<usize, Fault> {
    usize::try_from(value).map_err(|_| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 超出本平台 usize 表示范围"),
        )
    })
}

/// 规划一次自动化编辑（只读；`dryRun` 走的就是它）。
///
/// # Errors
///
/// - 没有活跃工程 → `NO_ACTIVE_PROJECT`（由调用方 `require_active` 给出）；
/// - 音轨/路由边/设备插槽/参数下标/宏下标不存在 → 模型错误映射（`TRACK_NOT_FOUND` /
///   `ENTITY_NOT_FOUND` / `INDEX_OUT_OF_BOUNDS`）；
/// - `lane`/`curve`/`edgeId`/下标形状非法 → `INVALID_PARAMETER_RANGE`；
/// - 值非有限或越出泳道有效取值域 → `OUT_OF_RANGE`；
/// - 模拟施加后模型校验失败 → 模型错误映射。
pub fn plan(
    project: &YebanProjectV1,
    arguments: &Map<String, Value>,
    track_id: EntityId,
) -> Result<AutomationEdit, Fault> {
    let (target, lane_kind, target_label) = parse_target(track_id, arguments)?;
    // 目标对账：模型自己的"这个目标在文档里真的存在吗"（错误是具体的）。
    target
        .validate_against(project)
        .map_err(|error| error::from_model("自动化目标对账", &error))?;

    let lane: Option<&AutomationLane> = project.automation_lane(&target);
    let points = lane
        .map(AutomationLane::points_in_tick_order)
        .unwrap_or_default();
    let read_enabled = lane.is_some_and(|lane| lane.read_enabled);
    let write_mode = lane.map(|lane| serde_name(&lane.write_mode));
    let unit = serde_name(&target.nominal_unit());
    let domain = lane
        .and_then(AutomationLane::effective_domain)
        .or_else(|| target.nominal_domain())
        .map(|domain| (domain.min(), domain.max()));

    // ---- 读的一半（保持调用方给的 tick 顺序） ----
    let read_ticks = parse_read_ticks(arguments)?;
    let mut values_before = Vec::with_capacity(read_ticks.len());
    for tick in &read_ticks {
        values_before.push(AutomationReading::read(project, &target, *tick)?);
    }

    let digest_before = digest_of(project)?;

    // ---- 写的一半（可选） ----
    let mut write = None;
    let mut values_after = None;
    let mut digest_after = None;
    if let Some(raw_point) = arguments.get("point") {
        let (point_id, point) = parse_point(arguments, raw_point, &target_label, domain)?;
        let old_point = lane.and_then(|lane| lane.points.get(&point_id)).copied();
        let op = Op::SetAutomationPoint {
            target,
            point_id,
            old_point,
            new_point: point,
        };
        // 模拟：整批必须能在**当前**工程上干净地施加，且施加后仍然合法。
        let mut simulated = project.clone();
        op.apply(&mut simulated)
            .map_err(|error| error::from_model("自动化点模拟", &error))?;
        simulated
            .validate()
            .map_err(|error| error::from_model("自动化点校验", &error))?;
        let mut after = Vec::with_capacity(read_ticks.len());
        for tick in &read_ticks {
            after.push(AutomationReading::read(&simulated, &target, *tick)?);
        }
        values_after = Some(after);
        digest_after = Some(digest_of(&simulated)?);
        write = Some(AutomationWrite {
            point_id,
            tick: point.tick,
            value: point.value,
            curve: point.curve,
            existed: old_point.is_some(),
            old_value: old_point.map(|old| old.value),
            op,
        });
    }

    Ok(AutomationEdit {
        track_id,
        target,
        target_label,
        lane_kind,
        points,
        lane_exists: lane.is_some(),
        read_enabled,
        write_mode,
        unit,
        domain,
        read_ticks,
        values_before,
        write,
        values_after,
        digest_before,
        digest_after,
    })
}

/// 读 `ticks` 实参（缺省空；每个元素必须是非负整数；条数有护栏）。
fn parse_read_ticks(arguments: &Map<String, Value>) -> Result<Vec<u64>, Fault> {
    let Some(value) = arguments.get("ticks") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ticks` 必须是整数数组",
        ));
    };
    if items.len() > MAX_READ_TICKS {
        let actual = items.len();
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`ticks` 最多 {MAX_READ_TICKS} 个 (实际 {actual}) —— 一次读几万个值会撑爆 Agent 上下文"
            ),
            serde_json::json!({ "field": "ticks", "limit": MAX_READ_TICKS, "actual": actual }),
        ));
    }
    let mut ticks = Vec::with_capacity(items.len());
    for item in items {
        ticks.push(item.as_u64().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`ticks` 的元素必须是非负整数, 实际收到 {item}"),
            )
        })?);
    }
    Ok(extension_pure::read_ticks(&ticks))
}

/// 解析 `point` 实参 → `(点身份, 模型点)`。
fn parse_point(
    arguments: &Map<String, Value>,
    raw: &Value,
    target_label: &str,
    domain: Option<(f32, f32)>,
) -> Result<(EntityId, AutomationPoint), Fault> {
    let object = raw.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`point` 必须是对象, 形如 {\"tick\": 0, \"value\": -6.0, \"curve\": \"Linear\"}",
        )
    })?;
    let tick = object.get("tick").and_then(Value::as_u64).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`point.tick` 必须是非负整数",
            serde_json::json!({ "field": "point.tick" }),
        )
    })?;
    let value = object.get("value").and_then(Value::as_f64).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`point.value` 必须是数字",
            serde_json::json!({ "field": "point.value" }),
        )
    })?;
    // f64 → f32: 值域校验在下一条, 这里只做宽度转换。
    #[allow(clippy::cast_possible_truncation)]
    let value = value as f32;
    if !value.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`point.value` 必须有限, 实际 {value}"),
            serde_json::json!({ "field": "point.value", "value": value }),
        ));
    }
    if let Some((min, max)) = domain
        && (value < min || value > max)
    {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`point.value` {value} 越出本泳道的有效取值域 [{min}, {max}]"),
            serde_json::json!({
                "field": "point.value",
                "value": value,
                "domainMin": min,
                "domainMax": max,
            }),
        ));
    }
    let curve = parse_curve(object.get("curve"), "point")?;
    let point_id = match arguments.get("pointId") {
        Some(raw) => {
            let text = raw.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`pointId` 必须是 26 字符 ULID 字符串",
                )
            })?;
            parse_entity("pointId", text)?
        }
        // 确定性派生: 同一 (目标, tick) ⇒ 同一点身份 ⇒ 重复写入是**更新**而不是堆积。
        None => deterministic_id(&extension_pure::point_label(target_label, tick)),
    };
    Ok((
        point_id,
        AutomationPoint {
            id: point_id,
            tick,
            value,
            curve,
        },
    ))
}

/// `serde` 派生名（`project.json` 里的写法），失败时给 `"unknown"`（绝不 panic）。
fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// 工程内容摘要（与 `Domain::project_digest` 同口径：规范化 JSON 的 SHA-256）。
fn digest_of(project: &YebanProjectV1) -> Result<String, Fault> {
    let json = super::store::serialize_project(project)?;
    Ok(super::store::digest_of(json.as_bytes()))
}

/// **施加**一次自动化编辑：提交那一条 op（若有），并校验预测与真实结果一致。
///
/// # Errors
///
/// - 没有活跃工程 → `NO_ACTIVE_PROJECT`；
/// - 提交被撤销会话拒绝 → 既有映射；
/// - 真实摘要与预览预测不一致 → `CONFLICT`（"预览说的"与"真做的"漂移）。
pub fn apply(
    domain: &mut super::Domain,
    edit: &AutomationEdit,
) -> Result<crate::tools::ToolResponse, Fault> {
    if let Some(write) = &edit.write {
        let now_ms = domain.now_ms();
        let commit = {
            let super::Domain {
                active,
                graph,
                undo,
                ..
            } = domain;
            let active = active.as_mut().ok_or_else(super::no_active_project)?;
            super::undo_session::commit(
                graph,
                &mut active.project,
                undo,
                super::CommitRequest {
                    now_ms,
                    // ⚠ 诚实缺口: 模型 `OpOrigin` 没有 `McpEdit` 变体（needs-1）。
                    // `AutomationRecord` = "自动化写入的落盘动作"，是最接近的一档；
                    // `McpProposal` 需要一个**真实存在**的提案，这里没有。
                    origin: OpOrigin::AutomationRecord,
                    message: format!(
                        "edit_automation {} tick {} = {}",
                        edit.target_label, write.tick, write.value
                    ),
                    ops: vec![write.op.clone()],
                },
            )
            .map_err(super::undo_refusal_to_fault)?
        };
        // 预测必须等于真实 —— 否则 `dryRun` 的预览就是一句空话。
        let project = domain
            .active_project()
            .ok_or_else(super::no_active_project)?;
        let actual = digest_of(project)?;
        if edit.digest_after.as_deref() != Some(actual.as_str()) {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                "自动化写入后的工程摘要与只读规划的预测不一致 (预览会撒谎)",
                serde_json::json!({
                    "predicted": edit.digest_after.clone(),
                    "actual": actual,
                    "commit": commit.to_canonical_string(),
                }),
            ));
        }
        // 读数同样重算一遍（不是把预览照抄回来）：真做的 `valuesAfter` 必须是
        // **在真实工程上**用唯一求值入口重新读出来的，并且与预测逐项相同。
        if let Some(predicted) = &edit.values_after {
            let project = domain
                .active_project()
                .ok_or_else(super::no_active_project)?;
            let mut actual_readings = Vec::with_capacity(edit.read_ticks.len());
            for tick in &edit.read_ticks {
                actual_readings.push(AutomationReading::read(project, &edit.target, *tick)?);
            }
            if &actual_readings != predicted {
                return Err(Fault::domain_with_data(
                    ErrorCode::Conflict,
                    "自动化写入后的读数与只读规划的预测不一致 (预览会撒谎)",
                    serde_json::json!({
                        "predicted": predicted.iter().map(|r| r.to_value()).collect::<Vec<_>>(),
                        "actual": actual_readings.iter().map(|r| r.to_value()).collect::<Vec<_>>(),
                        "commit": commit.to_canonical_string(),
                    }),
                ));
            }
        }
    }
    let mut data = edit.data()?;
    if let Value::Object(map) = &mut data {
        map.insert("applied".to_owned(), Value::from(edit.write.is_some()));
        map.insert("commitCount".to_owned(), Value::from(domain.commit_count()));
    }
    Ok(crate::tools::ToolResponse::success(data))
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

    fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn reading_uses_the_unique_entry_and_falls_back_to_the_static_value() {
        let project = filled_project();
        let track = lead(&project);
        // 样本的 lead 在 tick 0 上 -6.0 dB, tick 3840 上 0.0 dB。
        let plan = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("ticks", serde_json::json!([0, 1920, 3840])),
            ]),
            track,
        )
        .expect("规划");
        assert!(plan.is_read_only(), "没有 point ⇒ 一位都不改");
        assert_eq!(plan.values_before.len(), 3);
        assert_eq!(plan.values_before[0].automation_value, Some(-6.0));
        assert_eq!(plan.values_before[0].source, "automation");
        // tick 1920 = 区间中点: Linear 下 = -3.0 (模型唯一口径)。
        assert_eq!(plan.values_before[1].automation_value, Some(-3.0));
        assert_eq!(plan.values_before[1].effective_value, -3.0);
        assert_eq!(plan.values_before[2].automation_value, Some(0.0));
        assert_eq!(plan.unit, "Decibels");
        assert_eq!(plan.domain, Some((-60.0, 12.0)));
        assert_eq!(plan.points.len(), 2);
    }

    #[test]
    fn a_lane_without_points_reads_the_static_value_from_the_model() {
        let project = filled_project();
        let track = lead(&project);
        // 样本里**没有**声相泳道 ⇒ 求值入口给 None ⇒ 退回静态值 (模型唯一来源)。
        let plan = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackPan")),
                ("ticks", serde_json::json!([10])),
            ]),
            track,
        )
        .expect("规划");
        let reading = plan.values_before[0];
        assert_eq!(reading.automation_value, None);
        assert_eq!(reading.source, "static");
        assert!(!plan.lane_exists);
        assert_eq!(
            reading.static_value,
            project.track(&track).expect("音轨").pan
        );
    }

    #[test]
    fn writing_a_point_is_a_single_reversible_op() {
        let project = filled_project();
        let track = lead(&project);
        let plan = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramIndex", Value::from(0)),
                (
                    "point",
                    serde_json::json!({"tick": 960, "value": 250.0, "curve": "SCurve"}),
                ),
                ("ticks", serde_json::json!([960])),
            ]),
            track,
        )
        .expect("规划");
        let write = plan.write.as_ref().expect("必须有一个点");
        assert!(!write.existed, "样本里没有这个点");
        assert_eq!(write.old_value, None);
        assert_eq!(write.curve, CurveType::SCurve);
        assert_eq!(plan.ops().len(), 1);
        let mut simulated = project.clone();
        write.op.apply(&mut simulated).expect("施加");
        assert_eq!(
            simulated
                .automation_value_at(&plan.target, 960)
                .expect("对账"),
            Some(250.0)
        );
        // 逆操作回到起点（模型自己的 invert）。
        write.op.apply_inverse(&mut simulated).expect("撤销");
        assert_eq!(simulated, project, "逆操作必须逐字节回退");
    }

    #[test]
    fn rewriting_the_same_tick_updates_the_same_point() {
        let project = filled_project();
        let track = lead(&project);
        let base = args(&[
            ("lane", Value::from("TrackVolume")),
            ("point", serde_json::json!({"tick": 960, "value": -1.0})),
        ]);
        let first = plan(&project, &base, track).expect("规划");
        let mut after = project.clone();
        first
            .write
            .as_ref()
            .expect("点")
            .op
            .apply(&mut after)
            .expect("施加");
        let second = plan(&after, &base, track).expect("再规划");
        assert_eq!(
            first.write.as_ref().expect("点").point_id,
            second.write.as_ref().expect("点").point_id,
            "同一 (目标, tick) 必须派生出同一点身份"
        );
        assert!(second.write.as_ref().expect("点").existed);
        assert_eq!(second.write.as_ref().expect("点").old_value, Some(-1.0));
    }

    #[test]
    fn bad_shapes_and_unknown_names_use_contract_error_codes() {
        let project = filled_project();
        let track = lead(&project);
        // 未知 lane 名（别名）⇒ INVALID_PARAMETER_RANGE。
        let fault = plan(
            &project,
            &args(&[("lane", Value::from("trackVolume"))]),
            track,
        )
        .expect_err("别名必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        // SendGain 缺 edgeId。
        let fault = plan(&project, &args(&[("lane", Value::from("SendGain"))]), track)
            .expect_err("缺 edgeId");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        // 未知曲线名。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                (
                    "point",
                    serde_json::json!({"tick": 0, "value": 0.0, "curve": "sCurve"}),
                ),
            ]),
            track,
        )
        .expect_err("别名曲线必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        // 值域越界 ⇒ OUT_OF_RANGE（音量 12 dB 是上界）。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("point", serde_json::json!({"tick": 0, "value": 99.0})),
            ]),
            track,
        )
        .expect_err("越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        // 音轨不存在 ⇒ TRACK_NOT_FOUND（模型对账）。
        let ghost = deterministic_id("ghost");
        let fault = plan(
            &project,
            &args(&[("lane", Value::from("TrackVolume"))]),
            ghost,
        )
        .expect_err("音轨不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
        // 设备插槽越界 ⇒ INDEX_OUT_OF_BOUNDS。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(7)),
            ]),
            track,
        )
        .expect_err("插槽越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IndexOutOfBounds));
    }

    #[test]
    fn read_ticks_are_bounded_and_ordered() {
        let project = filled_project();
        let track = lead(&project);
        let many: Vec<u64> = (0..=u64::try_from(MAX_READ_TICKS).expect("小值")).collect();
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("ticks", serde_json::json!(many)),
            ]),
            track,
        )
        .expect_err("超过护栏");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));

        let plan = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("ticks", serde_json::json!([3840, 0])),
            ]),
            track,
        )
        .expect("规划");
        assert_eq!(plan.read_ticks, vec![3840, 0], "顺序必须保持");
        assert_eq!(plan.values_before[0].tick, 3840);
    }

    #[test]
    fn every_name_table_agrees_with_the_model_enum() {
        // 纯层的四个曲线名必须**恰好**是模型 CurveType 的 serde 名（不多不少）。
        for curve in [
            CurveType::Linear,
            CurveType::Exponential,
            CurveType::Logarithmic,
            CurveType::SCurve,
        ] {
            let name = curve_name(curve);
            assert!(
                extension_pure::CURVE_NAMES.contains(&name),
                "{curve:?} 的名字 {name} 不在纯层表里"
            );
            assert_eq!(parse_curve(Some(&Value::from(name)), "point"), Ok(curve));
        }
        assert_eq!(
            parse_curve(None, "point"),
            Ok(CurveType::Linear),
            "缺省是模型的默认曲线"
        );
    }
}
