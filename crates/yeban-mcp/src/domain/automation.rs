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
//! ## `DeviceParam` 的按名寻址（`paramName`）
//!
//! 目标寻址除 `slotIndex` + `paramIndex` 外还接受可选的 `paramName`
//! （与 `yeban_edit_notes` 的 `setParam` 同词同义）：只给名字 ⇒ 按该设备
//! 实例自己的 `params` 定下标；与 `paramIndex` 并存 ⇒ 两者必须指向同一个
//! 参数，不一致响亮拒绝。解析只有一份（[`resolve_device_param_address`]，
//! 两条写路径共用），落盘的目标仍然是纯下标形态 —— 工程文件的序列化
//! **不变**。回执以 `resolvedParamName` 回显落点规范名（与母带回执同名
//! 同口径），调用方不必靠下一次读回才发现指错了参数。
//!
//! ## 操作来源（`OpOrigin`）已**如实**（不再是借用）
//!
//! 模型 `OpOrigin` 现在有 [`OpOrigin::McpEdit`] —— "MCP 代理的直接编辑：不创建提案"。
//! 本工具正是这一档（它**不**创建提案，因此 [`OpOrigin::McpProposal`] 不适用），
//! 此前借用 [`OpOrigin::AutomationRecord`]（"自动化写入的落盘动作"）的写法已删除。
//! **作者**字段仍是 `yeban-mcp`（`UndoState.author`），因此不存在"伪装成用户操作"。
//!
//! **契约已承认**（第 82 轮起）：`schemas/ops.schema.json` 的 `origin.oneOf[2]` 就是 `McpEdit`
//! （`additionalProperties: false`），本线禁改 `schemas/**` ⇒ 漂移由
//! `yeban-model` 的显式欠账清单 `PENDING_CONTRACT_ORIGINS` 机械钉住；
//! 契约侧请求见 `docs/ledger/op-origin-mcp-notes.md`。

use serde_json::{Map, Value};

use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveType, DeviceDefinition, EntityId, Op,
    OpOrigin, YebanProjectV1,
};
use yeban_render::automation::LaneResidual;
use yeban_render::inserts::resolve_device_param;

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
    /// `DeviceParam` 目标落点的**规范参数名**（设备里存的原文）。
    ///
    /// 与母带回执 `data.automation[].resolvedParamName` 同一份口径：由
    /// `yeban_render::inserts::resolve_device_param`（唯一解析入口）给出，
    /// ⛔ 本层不自己拼。非 `DeviceParam` 目标为 `None`（回执里是 `null`）。
    /// 它的用处正是按名寻址的回读面：调用方给了 `paramName`（或下标）之后，
    /// 从回执就能核对自己到底写到了哪个参数，而不是靠下一次读回才发现指错。
    pub resolved_param_name: Option<String>,
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
    /// ⭐ 这次调用（写入后）的目标**在离线渲染里会不会被应用**。
    ///
    /// 口径与 `yeban_render_master` 的 `data.automation[].applied` 逐位相同：
    /// `evaluable && supported`。`supported` 来自
    /// [`yeban_render::automation::unsupported_lane_reason`]（与 `AutomationReport`
    /// 的 `supported` 同一份判定），`evaluable` = 泳道有点且 `readEnabled`。
    ///
    /// ⚠ 带 `point` 的调用看的是**写入后的模拟工程**（与 `valuesAfter` 同一次），
    /// 而 `lane` 块仍是写入**前**的快照 —— 因此新建泳道时可能出现
    /// `lane.pointCount == 0` 而本字段已 `true`：那个点正是本次写入。
    pub applied_in_offline_render: bool,
    /// 目标**没有落点**时的残差类（`None` = 有落点）。
    ///
    /// 这是 `residualReason` 的唯一来源；名字由 [`LaneResidual::as_str`] 给出
    /// （⛔ MCP 不另拼字面量）。当 `applied_in_offline_render == false` 而本字段
    /// 为 `None` 时，原因**不是**残差，而是"没有可求值点 / 读开关关闭"（导出同样
    /// 不登记 `automationLanes`），或探针编译失败（见 [`Self::offline_render_error`]）。
    pub residual_reason: Option<LaneResidual>,
    /// 探针计划编译失败的原因（`None` = 正常）。
    ///
    /// 失败时工程**根本导出不了**（同一处会 `RENDER_FAILED`）⇒ 本字段把"不知道"
    /// 与"没有落点"分开，⛔ 不拿一个残差类冒充。
    pub offline_render_error: Option<String>,
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
            // 与母带回执同名的落点名读数：`DeviceParam` 给规范名，其余目标 `null`。
            // 键**始终在**（与本响应其余键同一口径，⛔ 不静默省略）。
            "resolvedParamName": self.resolved_param_name.clone().map_or(Value::Null, Value::from),
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
            // ⭐ 离线渲染的落点读数：与导出响应 `data.automation` 同一份判定。
            // 键**始终在**（⛔ 不静默省略）：`false` + `residualReason:null` 的含义
            // 是"没有可求值点 / 读开关关闭"（见 `AutomationEdit` 的字段文档）。
            "appliedInOfflineRender": self.applied_in_offline_render,
            "residualReason": self.residual_reason.map(LaneResidual::as_str),
            "offlineRenderError": self.offline_render_error.clone().map_or(Value::Null, Value::from),
            "projectDigestBefore": self.digest_before.clone(),
            "projectDigestAfter": self.digest_after.clone().map_or(Value::Null, Value::from),
            "origin": {
                "kind": "McpEdit",
                "author": super::AGENT_NAME,
                "note": "MCP 直接编辑 (不创建提案) ⇒ OpOrigin::McpEdit",
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
/// `pub(crate)`：`#311c` 的 `note.slide.curve`（`domain::notes::read_slide`）**复用**
/// 这一份拼写/别名判定（名字表 `extension_pure::CURVE_NAMES`），⛔ 不复制第二套
/// —— 两份实现必然漂移（`ADR-0001` D28）。
///
/// # Errors
///
/// 未知名字 → `INVALID_PARAMETER_RANGE`。
pub(crate) fn parse_curve(raw: Option<&Value>, context: &str) -> Result<CurveType, Fault> {
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
    let controller = if kind.needs_controller() {
        if !arguments.contains_key("controller") {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`MidiCc` 目标必须给定 `controller` (MIDI 控制器号)",
            ));
        }
        let raw = parse_index(arguments, "controller")?;
        // 模型字段是 `u8`：>255 是**形状**错误，在本层拒绝；128..=255 能装进
        // `u8`，交给模型 `validate_against` 的既有 0..=127 判定拒绝。
        if raw > u64::from(u8::MAX) {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`controller` 超出 u8 表示范围: {raw}"),
                serde_json::json!({ "field": "controller", "value": raw }),
            ));
        }
        u8::try_from(raw).unwrap_or(u8::MAX)
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
        LaneKind::MidiCc => AutomationTarget::MidiCc {
            track_id: call_track,
            controller,
        },
    };
    let label = extension_pure::target_label(
        kind,
        &call_track.to_canonical_string(),
        edge.map(|edge| edge.to_canonical_string()).as_deref(),
        u64::from(u32::try_from(slot).unwrap_or(u32::MAX)),
        u64::from(u32::try_from(param).unwrap_or(u32::MAX)),
        u64::from(u32::try_from(macro_index).unwrap_or(u32::MAX)),
        u64::from(controller),
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

/// `DeviceParam` 按名寻址的实参名（`paramName`）。
///
/// 与 `yeban_edit_notes` 的 `setParam` 形态 `ops[].paramName` **逐字同词**
/// （`ADR-0001` D48）：同一个寻址概念在两个写面里必须同一个名字。
pub const PARAM_NAME_FIELD: &str = "paramName";

/// 一台设备上**全部**参数名（顺序 = 下标顺序）—— 错误报文里「可用名全集」的
/// 唯一构造处。名字来自设备实例自己的 `params`（渲染实际消费的那一份），
/// ⛔ 不从 `effect_params` 的识别名册或器件族字段表推导：那两份表回答的是
/// 「引擎认不认这个名字」，而寻址要回答的是「这个名字在这台设备上是第几个」。
fn device_param_names(device: &DeviceDefinition) -> Vec<String> {
    device
        .params
        .iter()
        .map(|param| param.name.clone())
        .collect()
}

/// 把 `DeviceParam` 的 (下标?, 名字?) 寻址解析成 (下标, 设备里存的规范名)。
///
/// **两条写路径共用这一份**（本工具的 [`plan`] 与 `yeban_edit_notes` 的
/// `setParam` 编译期）：名字→下标的对应关系只许在这里定义一次。匹配口径与
/// 全仓的参数名处理一致 —— **大小写不敏感**（`insertDevice` 的名字识别、
/// 渲染的 `strip_of` 投影都是这一口径），回给调用方的是设备里存的原文。
///
/// | 情形 | 结果 |
/// | :--- | :--- |
/// | 只给下标 | 下标在界内 ⇒ 原样返回（与旧行为逐字相同，判定在本函数内完成） |
/// | 只给名字 | 恰好命中一个 ⇒ 该下标；零个 ⇒ `paramNameNotFound`（报文带该设备可用名全集）；多个（同名参数）⇒ `paramNameAmbiguous`（带命中下标，请调用方用下标消歧） |
/// | 两者都给 | 下标处的名字与给定名字必须一致（大小写不敏感），否则 `paramNameIndexMismatch`（点名下标指向谁、名字指向谁） |
/// | 两者都不给 | `INVALID_PARAMETER_RANGE`（调用方应在解析期就拦住，本函数只作兜底） |
///
/// # Errors
///
/// - 音轨不存在 → `ENTITY_NOT_FOUND`（`reason = "trackNotFound"`，与 `setParam`
///   写路径同一字面量，⛔ 不新造词）；
/// - 插槽越界 → `OUT_OF_RANGE`（`reason = "slotOutOfRange"`，同上）；
/// - 下标越界 → `OUT_OF_RANGE`（`reason = "paramIndexOutOfRange"`，同上）；
/// - 名字类失败（不存在 / 同名歧义 / 与下标不一致）→
///   `INVALID_PARAMETER_RANGE`，`data` 带 `availableParamNames`（该设备全集）。
pub fn resolve_device_param_address(
    project: &YebanProjectV1,
    track_id: EntityId,
    slot_index: usize,
    param_index: Option<usize>,
    param_name: Option<&str>,
) -> Result<(usize, String), Fault> {
    let track_text = track_id.to_canonical_string();
    let track = project.tracks.get(&track_id).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("设备参数寻址失败: 音轨 {track_text} 不存在"),
            serde_json::json!({
                "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                "reason": "trackNotFound",
                "trackId": track_text,
            }),
        )
    })?;
    let device = track.devices.get(slot_index).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("设备参数寻址越界: 音轨 {track_text} 的第 {slot_index} 槽不存在"),
            serde_json::json!({
                "field": "slotIndex",
                "reason": "slotOutOfRange",
                "trackId": track_text,
                "slotIndex": slot_index,
            }),
        )
    })?;
    let device_id = device.id.to_canonical_string();
    let names = device_param_names(device);
    // 名字 → 全部命中下标（大小写不敏感；同名参数全部列出，⛔ 不偷偷取第一个）。
    let name_matches: Vec<usize> = param_name.map_or_else(Vec::new, |name| {
        names
            .iter()
            .enumerate()
            .filter(|(_, stored)| stored.eq_ignore_ascii_case(name))
            .map(|(index, _)| index)
            .collect()
    });
    let name_not_found = |name: &str| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "设备参数寻址失败: 音轨 {track_text} 第 {slot_index} 槽的设备 ({device_id}) \
                 没有名为 `{name}` 的参数"
            ),
            serde_json::json!({
                "field": PARAM_NAME_FIELD,
                "reason": "paramNameNotFound",
                "trackId": track_text,
                "slotIndex": slot_index,
                "deviceId": device_id,
                "paramName": name,
                "availableParamNames": names,
                "hint": "可用名字就是这台设备 `params` 的全集 (顺序即下标); 名字大小写不敏感",
            }),
        )
    };
    match (param_index, param_name) {
        (None, None) => Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "设备参数寻址必须给 `paramIndex` 或 `paramName` 至少一个",
        )),
        (Some(index), None) => {
            let stored = names.get(index).ok_or_else(|| {
                Fault::domain_with_data(
                    ErrorCode::OutOfRange,
                    format!(
                        "设备参数寻址越界: 音轨 {track_text} 第 {slot_index} 槽的设备\
                         没有第 {index} 个参数"
                    ),
                    serde_json::json!({
                        "field": "paramIndex",
                        "reason": "paramIndexOutOfRange",
                        "trackId": track_text,
                        "slotIndex": slot_index,
                        "paramIndex": index,
                    }),
                )
            })?;
            Ok((index, stored.clone()))
        }
        (None, Some(name)) => match name_matches.as_slice() {
            [] => Err(name_not_found(name)),
            [only] => Ok((*only, names[*only].clone())),
            multiple => Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "设备参数寻址有歧义: 音轨 {track_text} 第 {slot_index} 槽的设备 ({device_id}) \
                     有 {} 个参数都叫 `{name}`, 按名无法定到唯一一个",
                    multiple.len()
                ),
                serde_json::json!({
                    "field": PARAM_NAME_FIELD,
                    "reason": "paramNameAmbiguous",
                    "trackId": track_text,
                    "slotIndex": slot_index,
                    "deviceId": device_id,
                    "paramName": name,
                    "matchingIndices": multiple,
                    "availableParamNames": names,
                    "hint": "同名参数请用 `paramIndex` 点名下标消歧",
                }),
            )),
        },
        (Some(index), Some(name)) => {
            let at_index = names.get(index).ok_or_else(|| {
                Fault::domain_with_data(
                    ErrorCode::OutOfRange,
                    format!(
                        "设备参数寻址越界: 音轨 {track_text} 第 {slot_index} 槽的设备\
                         没有第 {index} 个参数"
                    ),
                    serde_json::json!({
                        "field": "paramIndex",
                        "reason": "paramIndexOutOfRange",
                        "trackId": track_text,
                        "slotIndex": slot_index,
                        "paramIndex": index,
                    }),
                )
            })?;
            if at_index.eq_ignore_ascii_case(name) {
                return Ok((index, at_index.clone()));
            }
            if name_matches.is_empty() {
                return Err(name_not_found(name));
            }
            Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "设备参数寻址自相矛盾: `paramIndex` {index} 指向参数 `{at_index}`, \
                     而 `paramName` `{name}` 指向下标 {name_matches:?}"
                ),
                serde_json::json!({
                    "field": PARAM_NAME_FIELD,
                    "reason": "paramNameIndexMismatch",
                    "trackId": track_text,
                    "slotIndex": slot_index,
                    "deviceId": device_id,
                    "paramIndex": index,
                    "paramNameAtIndex": at_index,
                    "paramName": name,
                    "paramNameIndices": name_matches,
                    "availableParamNames": names,
                    "hint": "`paramIndex` 与 `paramName` 并存时必须指向同一个参数; \
                             只保留一个即可",
                }),
            ))
        }
    }
}

/// 把 `paramName` 实参施加到 [`parse_target`] 的产物上（只对 `DeviceParam` 生效）。
///
/// `parse_target` 在**解析期**没有工程在手，名字→下标的解析只能在这里
/// （`plan` 有工程）做。`paramIndex` 是否**显式**给出决定语义：缺省时
/// `parse_target` 把下标填成 0，那个 0 是「没给」而不是「点名第 0 个」，
/// ⛔ 不能拿它与名字做一致性判定。
fn apply_param_name(
    project: &YebanProjectV1,
    target: AutomationTarget,
    kind: LaneKind,
    label: String,
    arguments: &Map<String, Value>,
) -> Result<(AutomationTarget, String), Fault> {
    let AutomationTarget::DeviceParam {
        track_id,
        slot_index,
        param_index,
    } = target
    else {
        return Ok((target, label));
    };
    if kind != LaneKind::DeviceParam {
        return Ok((target, label));
    }
    let Some(raw) = arguments.get(PARAM_NAME_FIELD) else {
        return Ok((target, label));
    };
    let name = raw.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PARAM_NAME_FIELD}` 必须是字符串, 实际收到 {raw}"),
        )
    })?;
    let explicit_index = arguments.contains_key("paramIndex").then_some(param_index);
    let (resolved_index, _) =
        resolve_device_param_address(project, track_id, slot_index, explicit_index, Some(name))?;
    let target = AutomationTarget::DeviceParam {
        track_id,
        slot_index,
        param_index: resolved_index,
    };
    let label = extension_pure::target_label(
        kind,
        &track_id.to_canonical_string(),
        None,
        u64::from(u32::try_from(slot_index).unwrap_or(u32::MAX)),
        u64::from(u32::try_from(resolved_index).unwrap_or(u32::MAX)),
        0,
        0,
    );
    Ok((target, label))
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
    // `paramName` 按名寻址（`DeviceParam` 专属）：名字→下标的解析需要工程在手，
    // 因此在解析期之后、对账之前完成；解析出的目标与纯下标目标无任何区别，
    // 之后的每一道判定（对账 / 求值 / 落点）都只看见下标形态。
    let (target, target_label) =
        apply_param_name(project, target, lane_kind, target_label, arguments)?;
    // 目标对账：模型自己的"这个目标在文档里真的存在吗"（错误是具体的）。
    target
        .validate_against(project)
        .map_err(|error| error::from_model("自动化目标对账", &error))?;
    // 落点规范名读数（回执 `resolvedParamName`）：对账已证明目标存在 ⇒
    // 唯一解析入口必然成功；`ok()` 只是把「已证明」写成类型上不需要再报一次错。
    let resolved_param_name = match target {
        AutomationTarget::DeviceParam {
            track_id: target_track,
            slot_index,
            param_index,
        } => resolve_device_param(project, target_track, slot_index, param_index)
            .ok()
            .map(|resolved| resolved.param_name),
        _ => None,
    };

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
    let offline_support = if let Some(raw_point) = arguments.get("point") {
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
        // ⭐ 落点读数必须看**写入之后**的工程（刚写的点让空泳道变成可求值）。
        offline_render_support(&simulated, &target)
    } else {
        offline_render_support(project, &target)
    };
    let (applied_in_offline_render, residual_reason, offline_render_error) = offline_support;

    Ok(AutomationEdit {
        track_id,
        target,
        target_label,
        lane_kind,
        resolved_param_name,
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
        applied_in_offline_render,
        residual_reason,
        offline_render_error,
    })
}

/// `(applied, residual, probe_error)` —— 写侧读数的**唯一**计算处。
///
/// - `applied` 与 `AutomationReport.applied` 同义：`evaluable && supported`；
/// - `supported` 与探针来自 `super::render`（`yeban_render` 的同一份判定，⛔ 不抄清单）；
/// - `evaluable` = 泳道存在 ∧ `readEnabled` ∧ 有点。
fn offline_render_support(
    project: &YebanProjectV1,
    target: &AutomationTarget,
) -> (bool, Option<LaneResidual>, Option<String>) {
    let status = super::render::offline_lane_status(project, target);
    let evaluable = project
        .automation_lane(target)
        .is_some_and(|lane| lane.read_enabled && !lane.points.is_empty());
    (evaluable && status.supported, status.residual, status.error)
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
            // `reason` 与本仓库其余闸门同一条口径：让"是**哪一道**闸门拒的"机器可读。
            // 这一道与下面那道值域闸门的契约码**相同**（都是 `OUT_OF_RANGE`），
            // 只有 `reason` / `domainMin` 能区分它们 —— 没有 `reason` 时判据只能去
            // 比错误文案（脆），而"非有限"与"越界"是两件不同的事。
            serde_json::json!({
                "field": "point.value",
                "value": value,
                "reason": "nonFiniteValue",
            }),
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
                    // `McpEdit` = "MCP 代理的直接编辑" —— 本工具不创建提案，
                    // 因此 `McpProposal` 不适用；这正是那一档。
                    origin: OpOrigin::McpEdit {
                        agent_name: super::AGENT_NAME.to_owned(),
                    },
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
    fn midi_cc_points_write_read_back_and_bounds_are_loud() {
        let project = filled_project();
        let track = lead(&project);
        // 写前读：没有 MidiCc 泳道 ⇒ 求值 None ⇒ 退回静态值 0.0
        // （模型 `static_value` 的 MidiCc 特判臂；`read_param` 本身拒绝它）。
        let before = plan(
            &project,
            &args(&[
                ("lane", Value::from("MidiCc")),
                ("controller", Value::from(64)),
                ("ticks", serde_json::json!([0])),
            ]),
            track,
        )
        .expect("写前规划");
        assert_eq!(
            before.target_label,
            format!("{}#cc64", track.to_canonical_string())
        );
        assert_eq!(before.values_before[0].automation_value, None);
        assert_eq!(before.values_before[0].static_value, 0.0);
        assert_eq!(before.values_before[0].effective_value, 0.0);
        assert_eq!(before.values_before[0].source, "static");
        assert_eq!(before.domain, Some((0.0, 127.0)));

        // 写一个点（延音踏板 CC64 = 100）⇒ 同一规划的写后读数就是该值。
        let planned = plan(
            &project,
            &args(&[
                ("lane", Value::from("MidiCc")),
                ("controller", Value::from(64)),
                ("point", serde_json::json!({"tick": 0, "value": 100.0})),
                ("ticks", serde_json::json!([0, 480])),
            ]),
            track,
        )
        .expect("写点规划");
        let write = planned.write.as_ref().expect("必须有一个点");
        let mut after = project.clone();
        write.op.apply(&mut after).expect("施加");
        // 带 ticks 的读模式回读一致（唯一求值入口）。
        let reread = plan(
            &after,
            &args(&[
                ("lane", Value::from("MidiCc")),
                ("controller", Value::from(64)),
                ("ticks", serde_json::json!([0, 480])),
            ]),
            track,
        )
        .expect("回读规划");
        assert_eq!(reread.values_before[0].automation_value, Some(100.0));
        assert_eq!(reread.values_before[0].effective_value, 100.0);
        assert_eq!(reread.values_before[1].automation_value, Some(100.0));
        // 渲染侧 CcProbe 消费同一条泳道（值钳到 0..=127 的 u8 表）。
        let mut probe = yeban_render::automation::CcProbe::new(&after, track, 48_000);
        assert_eq!(probe.controllers(), 1);
        let table = probe.table_at(0).expect("有 CC 泳道必须有表");
        assert_eq!(table[64], 100, "CC64 在 tick 0 的表值必须是写入的 100");
        assert_eq!(table[11], 0, "没有泳道的控制器读 0");
        // 逆操作逐字节回退。
        write.op.apply_inverse(&mut after).expect("撤销");
        assert_eq!(after, project);

        // controller 越界两段：128..=255 能装进 u8 ⇒ 模型对账拒绝；
        // >255 是形状错误 ⇒ 解析期拒绝；缺 controller ⇒ 解析期拒绝。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("MidiCc")),
                ("controller", Value::from(128)),
            ]),
            track,
        )
        .expect_err("128 必须被模型对账拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        for bad in [256_u64, 300] {
            let fault = plan(
                &project,
                &args(&[
                    ("lane", Value::from("MidiCc")),
                    ("controller", Value::from(bad)),
                ]),
                track,
            )
            .expect_err(">255 必须在解析期拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        }
        let fault = plan(&project, &args(&[("lane", Value::from("MidiCc"))]), track)
            .expect_err("缺 controller 必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        // 点值越出 [0, 127] ⇒ OUT_OF_RANGE。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("MidiCc")),
                ("controller", Value::from(64)),
                ("point", serde_json::json!({"tick": 0, "value": 128.0})),
            ]),
            track,
        )
        .expect_err("点值越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
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

    fn fault_data(fault: &Fault) -> &Value {
        match fault {
            Fault::Domain { data, .. } => data.as_ref().expect("本形态的失败必须带 data"),
            Fault::Impl { error } => panic!("应当是领域失败, 实际是 {error:?}"),
        }
    }

    /// 按名寻址与按下标记入落到**同一个目标**（含目标标签——点身份由标签派生，
    /// 标签不同就是写到两条泳道上）；回执 `resolvedParamName` 两种寻址都回显
    /// 设备里存的规范名，非 `DeviceParam` 目标为 `null`。
    #[test]
    fn param_name_addresses_the_same_slot_as_param_index() {
        let project = filled_project();
        let track = lead(&project);
        // 样本 lead 的第 0 槽设备: params[0] = cutoff, params[1] = resonance。
        let by_index = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramIndex", Value::from(1)),
            ]),
            track,
        )
        .expect("下标寻址");
        let by_name = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramName", Value::from("resonance")),
            ]),
            track,
        )
        .expect("按名寻址");
        assert_eq!(
            by_name.target, by_index.target,
            "名字与下标必须落到同一个目标"
        );
        assert_eq!(by_name.target_label, by_index.target_label);
        assert_eq!(by_name.resolved_param_name.as_deref(), Some("resonance"));
        assert_eq!(
            by_index.resolved_param_name.as_deref(),
            Some("resonance"),
            "纯下标调用也回显落点名"
        );
        // 大小写不敏感（与全仓名字口径一致），回显的是存名原文。
        let upper = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramName", Value::from("RESONANCE")),
            ]),
            track,
        )
        .expect("大写名字");
        assert_eq!(upper.target, by_index.target);
        assert_eq!(upper.resolved_param_name.as_deref(), Some("resonance"));
        // 回执键始终在: DeviceParam 给规范名, 其余目标给 null。
        let data = by_name.data().expect("回执");
        assert_eq!(data["resolvedParamName"], "resonance");
        let volume = plan(
            &project,
            &args(&[("lane", Value::from("TrackVolume"))]),
            track,
        )
        .expect("音量目标");
        assert_eq!(
            volume.data().expect("回执")["resolvedParamName"],
            Value::Null
        );
    }

    /// 按名寻址的三条失败都响亮，且报文带得够调用方一次改对：
    /// 不存在 ⇒ `paramNameNotFound` ＋ 可用名全集；同名 ⇒ `paramNameAmbiguous`
    /// ＋ 命中下标；双给不一致 ⇒ `paramNameIndexMismatch` 并点名两边指向谁。
    #[test]
    fn param_name_failures_are_loud_and_carry_the_available_names() {
        let project = filled_project();
        let track = lead(&project);
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramName", Value::from("cutoff_hz")),
            ]),
            track,
        )
        .expect_err("不存在的名字必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let data = fault_data(&fault);
        assert_eq!(data["reason"], "paramNameNotFound", "{data}");
        assert_eq!(
            data["availableParamNames"],
            serde_json::json!(["cutoff", "resonance"]),
            "{data}"
        );

        // 双给一致 ⇒ 通过, 且落点就是那个参数。
        let agreed = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramIndex", Value::from(1)),
                ("paramName", Value::from("resonance")),
            ]),
            track,
        )
        .expect("双给一致");
        assert_eq!(agreed.resolved_param_name.as_deref(), Some("resonance"));

        // 双给不一致 ⇒ 点名下标指向谁、名字指向谁。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramIndex", Value::from(0)),
                ("paramName", Value::from("resonance")),
            ]),
            track,
        )
        .expect_err("双给不一致必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let data = fault_data(&fault);
        assert_eq!(data["reason"], "paramNameIndexMismatch", "{data}");
        assert_eq!(data["paramNameAtIndex"], "cutoff", "{data}");
        assert_eq!(data["paramNameIndices"], serde_json::json!([1]), "{data}");

        // 同名参数 ⇒ 按名有歧义, 报文给命中下标让调用方消歧。
        let mut duplicated = project.clone();
        let device = &mut duplicated.tracks.get_mut(&track).expect("音轨").devices[0];
        let mut extra = device.params[1].clone();
        extra.value = 0.9;
        device.params.push(extra);
        let fault = plan(
            &duplicated,
            &args(&[
                ("lane", Value::from("DeviceParam")),
                ("slotIndex", Value::from(0)),
                ("paramName", Value::from("resonance")),
            ]),
            track,
        )
        .expect_err("同名参数按名必须报歧义");
        let data = fault_data(&fault);
        assert_eq!(data["reason"], "paramNameAmbiguous", "{data}");
        assert_eq!(data["matchingIndices"], serde_json::json!([1, 2]), "{data}");
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
    /// 读 tick 的**已发布上限是 256**，且闸门**含端点**（256 个放行、257 个拒绝）。
    ///
    /// 为什么单列这一条：既有判据 `read_ticks_are_bounded_and_ordered` 喂的是
    /// `0..=MAX_READ_TICKS`（257 个）—— 它同时用**常量**当输入，于是
    /// ① 把常量改小（256 → 128）读数照样红，常量本身从没进过判据；
    /// ② `== 上限` 这个**端点**从来没有被喂过，`>` 改成 `>=` 也照样绿。
    /// 第二轮注入实测：`AU-maxticks`（256 → 128）与 `AU-ticksgate`（`>` → `>=`）
    /// **双双全绿** —— 本判据就是为这两条写的（两条注入下本判据都变红）。
    #[test]
    fn the_read_tick_ceiling_is_256_and_includes_the_boundary() {
        // 字面值，不是常量自比：常量被改时本行必须红。
        assert_eq!(MAX_READ_TICKS, 256);
        let project = filled_project();
        let track = lead(&project);
        // 恰好 256 个 ⇒ 放行（端点在内）。
        let exactly: Vec<u64> = (0..256_u64).collect();
        let planned = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("ticks", serde_json::json!(exactly)),
            ]),
            track,
        )
        .expect("恰好 256 个 tick 必须放行 (闸门含端点)");
        assert_eq!(planned.read_ticks.len(), 256);
        // 257 个 ⇒ 响亮拒绝，且报文里报出**字面**上限。
        let too_many: Vec<u64> = (0..257_u64).collect();
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("ticks", serde_json::json!(too_many)),
            ]),
            track,
        )
        .expect_err("257 个 tick 必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        // 本文件没有公用的"取 data"助手，就在本判据里取（不改产线 API）。
        fn data_of(fault: &Fault) -> &Value {
            match fault {
                Fault::Domain { data, .. } => data.as_ref().expect("本形态的失败必须带 data"),
                Fault::Impl { error } => panic!("应当是领域失败, 实际是 {error:?}"),
            }
        }
        let data = data_of(&fault);
        assert_eq!(data["limit"], serde_json::json!(256), "上限必须是字面 256");
        assert_eq!(data["actual"], serde_json::json!(257));
    }

    /// 采样点值域的闸门**含两个端点**（`value == max` 与 `value == min` 都放行）。
    ///
    /// 第二轮注入实测：`AU-domain`（`value > max` → `value >= max`）**全绿** ——
    /// 既有判据只喂过 99.0（远在上界 12.0 之上），端点从未被喂过。端点是有意义的
    /// 一等输入（"把音量推到标称上限"是常见请求），因此本判据把它钉住。
    #[test]
    fn the_point_domain_gate_includes_both_end_points() {
        let project = filled_project();
        let track = lead(&project);
        // 音量的标称域 = [-60, 12]（同文件另一条判据已用字面值钉住）。
        for value in [-60.0_f64, 12.0] {
            let planned = plan(
                &project,
                &args(&[
                    ("lane", Value::from("TrackVolume")),
                    ("point", serde_json::json!({"tick": 0, "value": value})),
                ]),
                track,
            )
            .unwrap_or_else(|fault| panic!("端点 {value} 必须被接受, 实际 {fault:?}"));
            assert_eq!(planned.domain, Some((-60.0, 12.0)));
            let write = planned
                .write
                .as_ref()
                .expect("给了 `point` ⇒ 必然有写入载荷");
            assert_eq!(write.value, value as f32, "端点必须原样进写入载荷");
            assert_eq!(write.tick, 0);
        }
        // 端点之外一格 ⇒ OUT_OF_RANGE，报文报出**字面**端点。
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("point", serde_json::json!({"tick": 0, "value": 12.5})),
            ]),
            track,
        )
        .expect_err("12.5 dB 超过标称上界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let Fault::Domain { data, .. } = &fault else {
            panic!("应当是领域失败, 实际 {fault:?}");
        };
        let data = data.as_ref().expect("本形态的失败必须带 data");
        assert_eq!(data["domainMin"], serde_json::json!(-60.0));
        assert_eq!(data["domainMax"], serde_json::json!(12.0));
    }
    /// 非有限值在 **f64 → f32 收窄之后**被判，且这一道闸门与值域闸门**分得开**。
    ///
    /// 第二轮注入实测：`AU-finite`（把 `if !value.is_finite()` 换成 `if false`）
    /// **全绿** —— 没有任何判据喂过"JSON 文本层合法、收窄到 f32 之后是 `inf`"的输入。
    /// 机械前提（本机真跑过的探针）：`serde_json` 拒绝文本里的越界浮点
    /// （`"1e400"` / `"NaN"` 都是解析期错误，见 `serde_json-1.0.151/src/de.rs` 的
    /// `NumberOutOfRange`），但 **`1e39` 在 f64 里是有限数**、收窄到 `f32` 之后是 `inf`
    /// ⇒ 这一道闸门是**活的**，不是防御性死码（与 `ef459f9` 同一族）。
    #[test]
    fn a_point_value_that_only_overflows_after_narrowing_is_refused_as_non_finite() {
        let project = filled_project();
        let track = lead(&project);
        let fault = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("point", serde_json::json!({"tick": 0, "value": 1e39})),
            ]),
            track,
        )
        .expect_err("1e39 收窄到 f32 之后是 inf, 必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let Fault::Domain { data, .. } = &fault else {
            panic!("应当是领域失败, 实际 {fault:?}");
        };
        let data = data.as_ref().expect("本形态的失败必须带 data");
        assert_eq!(
            data["reason"], "nonFiniteValue",
            "必须是**非有限**那一道闸门拒的, 而不是值域闸门: {data}"
        );
        // 阴性对照: 值域闸门拒的报文**没有**这个 reason（两道闸门因此分得开）。
        let out_of_domain = plan(
            &project,
            &args(&[
                ("lane", Value::from("TrackVolume")),
                ("point", serde_json::json!({"tick": 0, "value": 12.5})),
            ]),
            track,
        )
        .expect_err("12.5 dB 越界");
        let Fault::Domain { data, .. } = &out_of_domain else {
            panic!("应当是领域失败, 实际 {out_of_domain:?}");
        };
        let data = data.as_ref().expect("带 data");
        assert!(data.get("reason").is_none(), "值域闸门不带这个 reason");
        assert_eq!(data["domainMax"], serde_json::json!(12.0));
    }
}
