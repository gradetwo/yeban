//! 自动化泳道求值、目标对账与取值域 [MODEL-AST-002, ARCH-DET-001]。
//!
//! ## 这是自动化求值的**唯一入口**
//!
//! 下游三条线（离线/实时渲染、音频引擎的音符调度与参数平滑、界面曲线绘制）
//! **不得**各自再写一份插值：
//!
//! - [`YebanProjectV1::automation_value_at`] —— 给目标 + tick，拿"此刻该用哪个值"；
//! - [`AutomationLane::value_at`] —— 已经拿到泳道时的纯计算版本（无 `Result`）；
//! - [`AutomationTarget::static_value`] —— 该泳道没有任何自动化值时的**静态值**
//!   （下游若自己读 `track.volume_db` / 设备参数，就又造出第二份事实源）；
//! - [`AutomationTarget::validate_against`] —— 目标在文档里是否真的存在。
//!
//! ## 求值口径（确定性）
//!
//! 采样点集合是 `BTreeMap<EntityId, AutomationPoint>`（键是点身份，不是 tick），
//! 因此求值前先按 `(tick, point_id)` 排序 —— 于是**同一 tick 上的多个点有一个确定的
//! 胜者**（`point_id` 字典序最大者），求值与"曲线端点"不会因 `BTreeMap` 的键序而不同。
//!
//! 设 `T` 为查询 tick，排序后的点为 `p₀…pₙ₋₁`，`a` 为 `tick <= T` 的**最后**一个点
//! （按 `(tick,id)` 序），`b` 为 `tick > T` 的**第一个**点：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 泳道没有采样点 | `None`（"无自动化值"，下游应退回静态值） |
//! | `T` 在首点之前 | 首点的值（**保持**，不外推） |
//! | `T` 在末点之后（含等于末点） | 末点的值（**保持**，不外推） |
//! | 只有一个点 | 该点的值（处处保持） |
//! | `T` 恰好落在某个点上 | 该点的值**逐位精确**（`t=0` ⇒ `u=0`） |
//! | `a.tick < T < b.tick` | `a.value + (b.value − a.value)·u`，`u = a.curve.ease(t)`，`t = (T−a.tick)/(b.tick−a.tick)` |
//!
//! 曲线形状的口径在 [`crate::CurveType::ease`]（只有 `+` `-` `*`，无超越函数 ⇒
//! 跨架构逐位可复现，ADR-0001 D32 的"IEEE 精确类零容差"）。
//!
//! ## 阶梯还是曲线？—— 规范措辞优先的裁决
//!
//! 规范把 `AutomationPoint::curve` 定义为"**到下一个点**的曲线形状"，也就是说
//! 采样点之间是**分段插值（曲线段）**，不是阶梯。本线因此**不新增**
//! `CurveType` 变体（规范列出的四个就是全集；`CurveType` 还被 `MidiNote.slide`
//! 复用，加变体会同时改动滑音语义）。"阶梯"只出现在边界：区间之外与单点情形为**保持**。

use crate::error::ModelError;
use crate::ids::EntityId;
use crate::project::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationUnit, AutomationValueDomain,
    YebanProjectV1,
};

/// 推子类目标（音轨音量、发送增益）的固有取值域下界 (dB)。
pub const NOMINAL_GAIN_MIN_DB: f32 = -60.0;

/// 推子类目标（音轨音量、发送增益）的固有取值域上界 (dB)。
pub const NOMINAL_GAIN_MAX_DB: f32 = 12.0;

impl AutomationTarget {
    /// 本目标所属的音轨身份（五个变体都有音轨）。
    ///
    /// 自动化泳道永远挂在音轨上（`: TrackV3::automation_lanes`），
    /// 因此这是"泳道在文档里的位置"的唯一推导口径。
    #[must_use]
    pub const fn track_id(&self) -> EntityId {
        match self {
            Self::TrackVolume { track_id }
            | Self::TrackPan { track_id }
            | Self::SendGain { track_id, .. }
            | Self::DeviceParam { track_id, .. }
            | Self::Macro { track_id, .. } => *track_id,
        }
    }

    /// 本目标的固有**单位**（总可判定）。
    #[must_use]
    pub const fn nominal_unit(&self) -> AutomationUnit {
        match self {
            Self::TrackVolume { .. } | Self::SendGain { .. } => AutomationUnit::Decibels,
            Self::TrackPan { .. } => AutomationUnit::Bipolar,
            Self::Macro { .. } => AutomationUnit::Normalized,
            Self::DeviceParam { .. } => AutomationUnit::Native,
        }
    }

    /// 本目标的固有**取值域**。
    ///
    /// 设备参数是唯一的例外：模型里没有任何地方声明过参数的合法区间
    /// （`ParameterValue` 只有一个自由文本 `unit`），所以这里诚实地返回 `None`
    /// （"取域不可知"）而不是编一个 `0..=1`。界面此时应当用曲线自身的最值自适应，
    /// 或由泳道的显式 `domain` 覆盖。
    #[must_use]
    pub fn nominal_domain(&self) -> Option<AutomationValueDomain> {
        let domain = match self {
            Self::TrackVolume { .. } | Self::SendGain { .. } => {
                AutomationValueDomain::pinned(NOMINAL_GAIN_MIN_DB, NOMINAL_GAIN_MAX_DB)
            }
            Self::TrackPan { .. } => AutomationValueDomain::pinned(-1.0, 1.0),
            Self::Macro { .. } => AutomationValueDomain::pinned(0.0, 1.0),
            Self::DeviceParam { .. } => return None,
        };
        Some(domain)
    }

    /// 与文档**对账**：目标指向的音轨/设备插槽/参数下标/宏下标/路由边是否真的存在。
    ///
    /// 这是"目标不存在时给出明确错误"的唯一判据入口：错误是**具体**的
    /// （哪个音轨 / 哪个插槽 / 哪条边），而不是笼统的"找不到东西"。
    ///
    /// # Errors
    ///
    /// - 音轨不存在 → [`ModelError::TrackNotFound`]；
    /// - `SendGain` 的路由边不存在 → [`ModelError::RoutingEdgeNotFound`]；
    /// - `DeviceParam` 的设备插槽/参数下标越界 → [`ModelError::DeviceSlotOutOfRange`] /
    ///   [`ModelError::ParamIndexOutOfRange`]；
    /// - `Macro` 的宏下标越界 → [`ModelError::MacroIndexOutOfRange`]。
    pub fn validate_against(&self, doc: &YebanProjectV1) -> Result<(), ModelError> {
        let track = doc.track(&self.track_id())?;
        match self {
            Self::TrackVolume { .. } | Self::TrackPan { .. } => Ok(()),
            Self::SendGain { edge_id, .. } => {
                if doc.routing_graph.edges.contains_key(edge_id) {
                    Ok(())
                } else {
                    Err(ModelError::RoutingEdgeNotFound { id: *edge_id })
                }
            }
            Self::DeviceParam {
                slot_index,
                param_index,
                ..
            } => {
                let device =
                    track
                        .devices
                        .get(*slot_index)
                        .ok_or(ModelError::DeviceSlotOutOfRange {
                            index: *slot_index,
                            len: track.devices.len(),
                        })?;
                if *param_index >= device.params.len() {
                    return Err(ModelError::ParamIndexOutOfRange {
                        index: *param_index,
                        len: device.params.len(),
                    });
                }
                Ok(())
            }
            Self::Macro { macro_index, .. } => {
                if *macro_index >= track.macros.len() {
                    return Err(ModelError::MacroIndexOutOfRange {
                        index: *macro_index,
                        len: track.macros.len(),
                    });
                }
                Ok(())
            }
        }
    }

    /// 该目标的**静态值**（"此刻没有自动化"时应使用的值）。
    ///
    /// 为什么要它：`SetAutomationPoint` 之前的下游若各自去读
    /// `track.volume_db` / `device.params[i].value`，就又造出第二份"目标 → 值"的映射。
    /// 这里刻意**不**复用 `SetParam` 的读法：`SendGain` 的取值是
    /// `Option<f32>`（`None` = 单位增益 = `0.0 dB`），而 `SetParam` 必须**拒绝**
    /// 发送增益以保住那个 `Option` 语义。
    ///
    /// # Errors
    ///
    /// - 音轨不存在 → [`ModelError::TrackNotFound`]；
    /// - `SendGain` 的路由边不存在 → [`ModelError::RoutingEdgeNotFound`]；
    /// - `DeviceParam` / `Macro` 下标越界 → 对应的下标错误。
    pub fn static_value(&self, doc: &YebanProjectV1) -> Result<f32, ModelError> {
        match self {
            Self::SendGain { edge_id, .. } => {
                let edge = doc
                    .routing_graph
                    .edges
                    .get(edge_id)
                    .ok_or(ModelError::RoutingEdgeNotFound { id: *edge_id })?;
                // `None` = 单位增益（0 dB）。这不是"猜"：`SetRoutingGain` 明确规定
                // `None` 表示单位增益，`Some(0.0)` 与它必须可区分。
                Ok(edge.gain_db.unwrap_or(0.0))
            }
            other => crate::ops::read_param(doc, *other),
        }
    }
}

impl AutomationLane {
    /// 采样点按 `(tick, point_id)` 升序的确定序列。
    ///
    /// 引擎/界面若要在热路径上反复求值，应当**缓存这一次排序的结果**并自己二分，
    /// 而不是每次调用 [`AutomationLane::value_at`]（那个是 O(n log n) 的模型层实现，
    /// 为的是"只有一个口径"，不是性能）。参见 `RSK-31`（自动化点密集轰炸）。
    #[must_use]
    pub fn points_in_tick_order(&self) -> Vec<AutomationPoint> {
        let mut ordered: Vec<AutomationPoint> = self.points.values().copied().collect();
        ordered.sort_by_key(|point| (point.tick, point.id));
        ordered
    }

    /// `tick` 处的自动化值；`None` 表示"本泳道没有值"（空泳道）。
    ///
    /// 口径（边界行为）见模块文档；纯函数、无分配之外的副作用、**确定性**：
    /// 同一泳道同一 tick 两次求值逐位相同。
    #[must_use]
    pub fn value_at(&self, tick: u64) -> Option<f32> {
        let ordered = self.points_in_tick_order();
        let (Some(first), Some(last)) = (ordered.first(), ordered.last()) else {
            return None;
        };
        // 首点之前 / 末点之后：**保持**（不外推）。
        if tick < first.tick {
            return Some(first.value);
        }
        if tick >= last.tick {
            return Some(last.value);
        }
        // `partition_point` 返回第一个 `tick > T` 的下标；因此 `low` 是最后一个
        // `tick <= T` 的点（同 tick 时 `point_id` 最大者），`high` 严格在其后。
        let upper = ordered.partition_point(|point| point.tick <= tick);
        let low = &ordered[upper - 1];
        let high = &ordered[upper];
        Some(interpolate(low, high, tick))
    }

    /// 本泳道的**单位**（由目标派生，恒可判定）。
    #[must_use]
    pub const fn unit(&self) -> AutomationUnit {
        self.target.nominal_unit()
    }

    /// 本泳道的**有效取值域**：泳道显式声明的覆盖优先，否则取目标的固有值域。
    #[must_use]
    pub fn effective_domain(&self) -> Option<AutomationValueDomain> {
        self.domain.or_else(|| self.target.nominal_domain())
    }
}

/// 在 `low`/`high` 之间按 `low.curve` 插值；调用者保证 `low.tick < high.tick`。
fn interpolate(low: &AutomationPoint, high: &AutomationPoint, tick: u64) -> f32 {
    debug_assert!(high.tick > low.tick, "插值区间必须非空");
    let span = high.tick - low.tick;
    let offset = tick - low.tick;
    let t = offset as f32 / span as f32;
    let eased = low.curve.ease(t);
    low.value + (high.value - low.value) * eased
}

impl YebanProjectV1 {
    /// 取出目标对应的自动化泳道；音轨不存在或该目标没有泳道都返回 `None`。
    #[must_use]
    pub fn automation_lane(&self, target: &AutomationTarget) -> Option<&AutomationLane> {
        self.tracks
            .get(&target.track_id())
            .and_then(|track| track.automation_lanes.get(target))
    }

    /// **下游唯一求值入口**：目标在 `tick` 处的自动化值。
    ///
    /// 返回值的三种语义：
    /// - `Err(_)`：目标本身在文档里不存在（[`AutomationTarget::validate_against`]）；
    /// - `Ok(None)`：目标存在，但**没有**可用的自动化值（无泳道 / 空泳道 /
    ///   泳道 `read_enabled == false`）⇒ 调用方应退回
    ///   [`AutomationTarget::static_value`]；
    /// - `Ok(Some(value))`：该 tick 处的自动化值。
    ///
    /// # Errors
    ///
    /// 目标对账失败时返回具体错误（见 [`AutomationTarget::validate_against`]）。
    pub fn automation_value_at(
        &self,
        target: &AutomationTarget,
        tick: u64,
    ) -> Result<Option<f32>, ModelError> {
        target.validate_against(self)?;
        let Some(lane) = self.automation_lane(target) else {
            return Ok(None);
        };
        if !lane.read_enabled {
            return Ok(None);
        }
        Ok(lane.value_at(tick))
    }
}
