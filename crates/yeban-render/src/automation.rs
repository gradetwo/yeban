//! 离线导出里的**音量 / 发送增益 / 声相自动化**：目标求值 → 乘子或声相位置 → 5 ms
//! 平滑 [ARCH-DSP-001]。
//!
//! ## 这一层只做三件事
//!
//! 1. **求值**：每块（[`AUTOMATION_BLOCK_SIZE`] = 128 帧）调一次模型层的**唯一入口**
//!    [`YebanProjectV1::automation_value_at`]（目标对账 + `read_enabled` 闸 + 分段曲线
//!    插值都在那边，本模块**不**复制任何插值）；
//! 2. **平滑**：复用 `yeban_dsp::smoothing::ParamSmoother`（规范时间常数 5 ms），
//!    块内逐样本走一阶低通 —— 所以导出的不是监听侧 60 Hz 的 16.7 ms 阶梯，
//!    块级目标 + 样本级平滑是 S1 口径；
//! 3. **换域**（R55）：自动化值是 **dB**，而源输出已经含静态推子
//!    `db_to_linear(track.volume_db)`，因此这里给出的是**乘子**
//!    `db_to_linear(自动化 dB) / db_to_linear(track.volume_db)`。
//!    ⛔ 分母**不是** `mute`/`solo` 之后的轨增益（那个在静音时是 `0.0`）：
//!    把它当分母会除零，或者把静音轨“复活”。
//!    ⚠ **声相不走 R55**：它的值域是位置（`-1 ..= 1`），平滑的是位置本身，乘的是
//!    [`centered_pan_gains`] 给出的**绝对**左右增益（见 [`TrackPanAutomation`]）。
//!
//! ## 恒等快路径（一个样本都不碰）
//!
//! 没有泳道、`read_enabled == false`、泳道没有点，或者**每个点的值都等于静态值**
//! （音量/发送比的是线性增益的位型，声相比的是位置的位型，设备参数比的是**钳制后**
//! 的 dB 位型）⇒ `is_identity` 为真，整段跳过。
//! 这与 `crates/yeban-engine/src/param.rs` 的“整段跳过”是同一口径：不是乘 `1.0`
//! （那会动 `-0.0` 的符号与次正规位型），而是**一条样本都不读不写**。
//!
//! ## 为什么求值器是一个“只含本轨的模型工程”
//!
//! 源的 trait 对象要求 `'static`（[`crate::render::AudioSource`]），而离线导出的工程
//! 是借来的；把 `&YebanProjectV1` 塞进源会逼着整条渲染链加生命周期参数。这里存一份
//! **只含本轨（及其泳道）的最小工程**，每块对它调**同一个** `automation_value_at`。
//! 求值语义没有第二份：泳道数据是构造期从原工程克隆的，之后只读。
//!
//! ## 边界（本模块没有证明什么）
//!
//! - 只接 [`AutomationTarget::TrackVolume`]（含主总线）、[`AutomationTarget::SendGain`]、
//!   [`AutomationTarget::TrackPan`] 与 [`AutomationTarget::DeviceParam`] 里**插入链能改的
//!   标量字段**（[`APPLIED_DEVICE_PARAM_NAMES`]：41 个拼写 = 22 个字段，含别名；
//!   通道条 14 ＋ Freeverb 4 ＋ 卷积标量 4）；**压缩**与**卷积 IR 形状**
//!   （`conv_ir_decay*` / `conv_ir_seed`）如实登记（⛔ 不冒充已支持）；
//! - `#297` 起 [`AutomationTarget::Macro`] **接上了离线渲染**，口径是
//!   `目标当前值 + depth × macro_value`（`depth` 是目标单位的振幅）。只有**存在
//!   `Macro` 泳道**时才做动态级联：没有宏泳道的静态宏不参与渲染（`yeban_set_macro`
//!   已经把级联物化进目标泳道，再算一次就是双计）。⚠ 若同一个宏**既有** `Macro`
//!   泳道、又已被 `set_macro` 物化，两条路径会叠加（本单未去重；现有夹具无此组合）；
//! - 设备参数自动化的形状**不同**（任务单 `#266`）：它改的是**器件参数**、要重算系数
//!   ⇒ 只在**块边界**调 [`crate::inserts::InsertRuntime::apply_device_params`]，
//!   ⛔ 不逐样本平滑，也 ⛔ 不乘样本；单位是**原生绝对值**（`_db` = dB、`_hz` = Hz、
//!   秒 = 秒、开关 `0.0`/`1.0`），没有 R55 换域；混响/卷积的 128 帧块边界改参数
//!   **没有平滑**（与既有 `DeviceParam` 同一条纪律）⇒ 本模块只保证“字节变了 ＋
//!   分段读数”，⛔ 不证听感；
//! - 主总线的第二个站点在调用方（主总线没有源，它的 `volume_db` 在源侧无法表达）；
//! - 发送量的落点在 [`crate::render::RenderPlan`] 的**入边归约**里（见
//!   [`SendGainAutomation`]）：它乘的是该边的**静态增益**，与源节点的 `TrackVolume`
//!   是两个不同的站点，互不叠加；
//! - 声相（[`TrackPanAutomation`]）的落点在调用方的**逐轨源输出之后**：调用方必须在
//!   非恒等时把源侧静态声相关掉（`pan = 0` ⇒ `(1, 1)`），否则会乘两次 —— 本模块只
//!   提供那个乘子，⛔ 不替调用方改源；
//! - 没有把“自动化值”写回快照或模型：这是**离线渲染的一次性读数**。

use std::collections::{BTreeMap, BTreeSet};

use yeban_dsp::channel_strip::ChannelStripParams;
use yeban_dsp::smoothing::ParamSmoother;
use yeban_model::{AutomationLane, AutomationTarget, EntityId, PPQ, TrackV3, YebanProjectV1};

use crate::inserts::{
    DeviceParamFamily, DeviceParamField, ForcedInsertInstances, convolution_source_params,
    convolution_source_slot, forced_identity_strip, resolve_device_param, reverb_source_params,
    reverb_source_slot, strip_source_params, strip_source_slot,
};
use crate::render::{L1_BLOCK_SIZE, db_to_linear};

/// 自动化求值的块大小（帧）。与 [ARCH-DET-001] 的统一处理块一致。
pub const AUTOMATION_BLOCK_SIZE: usize = L1_BLOCK_SIZE;

/// 把母带帧号换成该帧起点的 tick（整数向下取整）。
///
/// 这是 `tick → frame` 的**逆**方向：`frame * PPQ * bpm / (sample_rate * 60)`。
/// 用 `f64` 是因为 `bpm` 本身就是 `f64`，且块起点只用于**块级**求值 —— 一个块最多
/// 128 帧（2.7 ms @48 kHz）的量化误差，落在 S1 的“块级取一次”口径内。
#[must_use]
pub fn tick_at_frame(frame: u64, bpm: f64, sample_rate: u32) -> u64 {
    if !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ticks = (frame as f64) * (PPQ as f64) * bpm / (f64::from(sample_rate) * 60.0);
    if !ticks.is_finite() || ticks <= 0.0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss)]
    if ticks >= u64::MAX as f64 {
        return u64::MAX;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tick = ticks.floor() as u64;
    tick
}

/// 一条 `Macro` 泳道对**某个目标**的级联贡献（任务单 `#297`）。
///
/// 只由**存在的 `Macro` 泳道**产生：没有 `Macro` 泳道的静态宏继续不参与渲染 ——
/// `yeban_set_macro` 已经把级联**物化**进目标泳道，再无条件算一次就是同一份映射
/// **双计**。因此本类型只在"目标有一条映射到它的 `Macro` 泳道"时才非空。
#[derive(Clone, Debug)]
struct MacroLaneSource {
    /// 映射振幅，**目标单位**（[`yeban_model::MacroMapping::depth`]）。
    depth: f32,
    /// 宏位置曲线（`0.0..=1.0`；`value_at` 是模型层的唯一插值）。
    lane: AutomationLane,
}

/// 一个目标上的全部宏级联贡献（`#297`）。
///
/// 求值口径逐字照裁决：`获得值 = 目标静态值 + depth × macro_value`。
/// 同一目标被多条宏泳道映射时**相加**（每条映射都是独立的一路摆动）。
#[derive(Clone, Debug, Default)]
struct MacroCascade {
    /// 每一路映射的振幅与宏曲线。
    sources: Vec<MacroLaneSource>,
}

impl MacroCascade {
    /// 扫描工程里**全部 `Macro` 泳道**，收集映射到 `target` 的贡献。
    ///
    /// - 只认 `read_enabled` 且有点的宏泳道（关掉/空泳道 = 此刻没有宏值）；
    /// - 目标为 `Macro` 的映射**不在这里递归**：嵌套宏本切片不消费，判定侧
    ///   （[`unsupported_lane_reason`]）也把它报成 `macro`；
    /// - 只收集 `depth != 0.0` 的映射（`0.0` 的摆动恒为 0，收进来只会多一次乘法）。
    fn for_target(project: &YebanProjectV1, target: &AutomationTarget) -> Self {
        let mut sources = Vec::new();
        for track in project.tracks.values() {
            for (lane_target, lane) in &track.automation_lanes {
                let AutomationTarget::Macro {
                    track_id,
                    macro_index,
                } = lane_target
                else {
                    continue;
                };
                if !lane.read_enabled || lane.points.is_empty() {
                    continue;
                }
                let Some(macro_track) = project.tracks.get(track_id) else {
                    continue;
                };
                let Some(macro_parameter) = macro_track.macros.get(*macro_index) else {
                    continue;
                };
                for mapping in &macro_parameter.mappings {
                    if mapping.target == *target
                        && !matches!(mapping.target, AutomationTarget::Macro { .. })
                        && mapping.depth != 0.0
                    {
                        sources.push(MacroLaneSource {
                            depth: mapping.depth,
                            lane: lane.clone(),
                        });
                    }
                }
            }
        }
        Self { sources }
    }

    /// 这一目标上是否有宏级联（哪怕此刻所有点都是 0）。
    fn has_sources(&self) -> bool {
        !self.sources.is_empty()
    }

    /// 级联在 `tick` 处的偏移（**目标单位**）。
    fn offset_at(&self, tick: u64) -> f32 {
        let mut offset = 0.0_f32;
        for source in &self.sources {
            if let Some(value) = source.lane.value_at(tick)
                && value.is_finite()
            {
                offset += source.depth * value;
            }
        }
        offset
    }

    /// 级联是否**处处**为 0（⇒ 目标可与没有宏泳道时逐位相同）。
    ///
    /// 分段插值在两端的点都为 0 时整段恒 0（边界是保持）⇒ 所有点都是 `0.0`
    /// 就是充要条件。
    fn is_zero(&self) -> bool {
        self.sources
            .iter()
            .all(|source| source.lane.points.values().all(|point| point.value == 0.0))
    }

    /// 宏曲线上的点数（读数用）。
    fn points(&self) -> usize {
        self.sources
            .iter()
            .map(|source| source.lane.points.len())
            .sum()
    }
}

/// 工程里是否有**可求值**的宏泳道映射到 `target`（任务单 `#297`）。
///
/// 调用方用它决定"目标自己没有泳道时还要不要挂自动化"（总线推子等）；
/// 扫描器共享的是同一个 [`MacroCascade`] 实现，⛔ 不各自再拼一份映射表。
#[must_use]
pub fn macro_cascade_reaches(project: &YebanProjectV1, target: &AutomationTarget) -> bool {
    MacroCascade::for_target(project, target).has_sources()
}

/// 一条音轨的音量自动化（`TrackVolume`）。
///
/// 用法（两个导出共用）：
///
/// ```text
/// let mut automation = TrackGainAutomation::new(project, track_id, sample_rate);
/// if !automation.is_identity() {
///     automation.apply_ratio_block(&mut out, first_frame, frames, channels);
/// }
/// ```
///
/// `apply_ratio_block` 对 `out` 乘的是 **R55 比率**；对主总线还多一个
/// [`Self::apply_gain_block`]，它把静态主推子一起乘进来（主总线没有源可乘）。
#[derive(Clone, Debug)]
pub struct TrackGainAutomation {
    /// 被自动化的目标（永远是 `TrackVolume`）。
    target: AutomationTarget,
    /// 静态推子的 dB 值（`TrackV3::volume_db`）。
    static_db: f32,
    /// 静态推子的线性增益 = `db_to_linear(static_db)`。
    static_gain: f32,
    /// 曲线是否**处处**等于静态推子 ⇒ 恒等快路径。
    identity: bool,
    /// 只含本轨的最小求值工程（见模块文档）。
    evaluator: YebanProjectV1,
    /// 5 ms 一阶低通。
    smoother: ParamSmoother,
    /// 工程速度（tick ↔ 帧）。
    bpm: f64,
    /// 采样率。
    sample_rate: u32,
    /// 泳道是否可求值（有点且 `read_enabled`）。
    evaluable: bool,
    /// 泳道的点数（读数用；恒等时也保留）。
    points: usize,
    /// 映射到本目标的宏级联（`#297`；空 = 没有 `Macro` 泳道映射到它）。
    macro_cascade: MacroCascade,
    /// 已被求值的块数（读数用）。
    blocks: u64,
    /// 最近一次块级曲线值（dB）。
    last_db: Option<f32>,
}

impl TrackGainAutomation {
    /// 构造一条轨的音量自动化求值器。
    ///
    /// 没有泳道 / `read_enabled == false` / 空泳道 / 每个点的线性增益都等于静态推子
    /// ⇒ [`Self::is_identity`] 为真（调用方应整段跳过）。
    #[must_use]
    pub fn new(project: &YebanProjectV1, track_id: EntityId, sample_rate: u32) -> Self {
        Self::with_target(
            project,
            AutomationTarget::TrackVolume { track_id },
            track_id,
            sample_rate,
        )
    }

    /// 两个目标（`TrackVolume` / `SendGain`）**共用**的构造器。
    ///
    /// 唯一的差别是“静态值从哪里读”：`TrackVolume` 来自 `TrackV3::volume_db`，
    /// `SendGain` 来自 `RoutingEdge::gain_db`（`None` = 单位增益 = 0 dB）。求值、
    /// 平滑与恒等判定**没有**第二份实现 —— 这正是 [`SendGainAutomation`] 复用它的原因。
    fn with_target(
        project: &YebanProjectV1,
        target: AutomationTarget,
        track_id: EntityId,
        sample_rate: u32,
    ) -> Self {
        let static_db = match target {
            AutomationTarget::TrackVolume { .. } => project
                .tracks
                .get(&track_id)
                .map_or(0.0, |track| track.volume_db),
            AutomationTarget::SendGain { edge_id, .. } => project
                .routing_graph
                .edges
                .get(&edge_id)
                .map_or(0.0, |edge| edge.gain_db.unwrap_or(0.0)),
            // 本模块不构造其它目标；退回单位增益并让恒等快路径兜住。
            AutomationTarget::TrackPan { .. }
            | AutomationTarget::DeviceParam { .. }
            | AutomationTarget::Macro { .. } => 0.0,
        };
        let static_gain = db_to_linear(static_db);
        let target_exists = match target {
            AutomationTarget::SendGain { edge_id, .. } => {
                project.routing_graph.edges.contains_key(&edge_id)
            }
            _ => true,
        };
        let lane = project.automation_lane(&target);
        let macro_cascade = MacroCascade::for_target(project, &target);
        let (evaluable, points, lane_identity) = match lane {
            None => (false, 0, true),
            Some(lane) if !lane.read_enabled => (false, lane.points.len(), true),
            Some(lane) if lane.points.is_empty() => (false, 0, true),
            Some(lane) => {
                // “曲线值处处等于静态值”的充要条件：每个采样点上的**线性增益**都与
                // 静态推子逐位相同。分段插值在两端相等时整段恒定，边界是保持；
                // 因此只要所有点相等，整条曲线就处处相等。
                let identity = !target_exists
                    || lane
                        .points
                        .values()
                        .all(|point| db_to_linear(point.value).to_bits() == static_gain.to_bits());
                (target_exists, lane.points.len(), identity)
            }
        };
        // `#297`：宏级联是非恒等的一条独立来源 —— 即便目标自己没有泳道，
        // 只要有一条 `Macro` 泳道映射到它，曲线就不再处处等于静态值。
        // 全部宏点都是 0 时偏移恒 0 ⇒ 仍走恒等快路径（正对照的机械保证）。
        let identity = !target_exists || (lane_identity && macro_cascade.is_zero());
        let evaluable = target_exists && (evaluable || macro_cascade.has_sources());
        let points = points + macro_cascade.points();
        // 最小求值工程：只放本轨，泳道键/值原样克隆（`automation_value_at` 的目标
        // 对账只看 `tracks` 里有没有这条轨）。没有泳道时也放一条空轨，保证目标对账
        // 与真实工程同解。`SendGain` 还要求那条**路由边**存在，因此克隆它（逐字段，
        // 不改原工程）；没有这条边时 `target_exists` 已是 `false`，构造的工程不会
        // 被求值（恒等快路径）。
        let mut evaluator = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            volume_db: static_db,
            ..TrackV3::default()
        };
        if let Some(track_ref) = project.tracks.get(&track_id) {
            track.automation_lanes = track_ref.automation_lanes.clone();
        }
        evaluator.tracks.insert(track_id, track);
        if let AutomationTarget::SendGain { edge_id, .. } = target
            && let Some(edge) = project.routing_graph.edges.get(&edge_id)
        {
            evaluator.routing_graph.edges.insert(edge_id, *edge);
        }
        #[allow(clippy::cast_precision_loss)]
        let smoother = ParamSmoother::with_default_time(sample_rate as f32);
        let mut automation = Self {
            target,
            static_db,
            static_gain,
            identity,
            evaluator,
            smoother,
            bpm: project.bpm,
            sample_rate,
            evaluable,
            points,
            macro_cascade,
            blocks: 0,
            last_db: None,
        };
        // 首块的曲线值就是起点：不从这里“淡入”，否则 tick 0 处的自动化值会被
        // 5 ms 的上升沿改写（那是监听侧才需要的防咔嗒行为；离线母带从 0 帧开始，
        // 曲线在 0 帧的值就是作者写的值）。
        automation.snap_to_start();
        automation
    }

    /// 把平滑器直接置于 tick 0 的曲线值（构造与复位共用）。
    fn snap_to_start(&mut self) {
        if self.identity {
            return;
        }
        if let Some(value) = self.composed_db_at(0) {
            let ratio = automation_ratio(value, self.static_gain);
            self.smoother.snap_to(ratio);
        }
    }

    /// 本目标在 `tick` 处的**有效 dB**：目标泳道（若有）＋ 宏级联偏移。
    ///
    /// - 目标有可求值泳道 ⇒ 基值 = 泳道值；
    /// - 没有（无泳道 / 关掉 / 空泳道）但存在宏级联 ⇒ 基值 = 静态值；
    /// - 两者都没有 ⇒ `None`（下游走静态快路径）。
    fn composed_db_at(&self, tick: u64) -> Option<f32> {
        let lane_value = match self.evaluator.automation_value_at(&self.target, tick) {
            Ok(Some(value)) if value.is_finite() => Some(value),
            _ => None,
        };
        let offset = self.macro_cascade.offset_at(tick);
        let value = match lane_value {
            Some(value) => value + offset,
            None if self.macro_cascade.has_sources() => self.static_db + offset,
            None => return None,
        };
        value.is_finite().then_some(value)
    }

    /// 复位到“全新构造”的状态（同一个计划连续执行时调用）。
    ///
    /// 求值器本身只读、无跨执行状态；需要复位的是 5 ms 平滑器与读数计数器。
    pub fn reset_for_execution(&mut self) {
        self.blocks = 0;
        self.last_db = None;
        self.snap_to_start();
    }

    /// 曲线是否**处处**等于静态推子（⇒ 一个样本都不用碰）。
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.identity
    }

    /// 泳道是否有可求值点（有点且 `read_enabled`）。
    #[must_use]
    pub const fn is_evaluable(&self) -> bool {
        self.evaluable
    }

    /// 泳道的点数。
    #[must_use]
    pub const fn points(&self) -> usize {
        self.points
    }

    /// 静态推子的 dB 值。
    #[must_use]
    pub const fn static_db(&self) -> f32 {
        self.static_db
    }

    /// 静态推子的线性增益。
    #[must_use]
    pub const fn static_gain(&self) -> f32 {
        self.static_gain
    }

    /// 已被求值的块数（只统计**非恒等**路径）。
    #[must_use]
    pub const fn evaluated_blocks(&self) -> u64 {
        self.blocks
    }

    /// 最近一次块级曲线值（dB）；恒等路径不更新。
    #[must_use]
    pub const fn last_db(&self) -> Option<f32> {
        self.last_db
    }

    /// 对源输出乘 R55 比率（源里已经含静态推子）。
    ///
    /// 恒等 ⇒ **整段跳过**（一个样本都不碰）。
    pub fn apply_ratio_block(
        &mut self,
        out: &mut [f32],
        first_frame: u64,
        frames: usize,
        channels: usize,
    ) {
        if self.identity || channels == 0 {
            return;
        }
        let frames = frames.min(out.len() / channels);
        if frames == 0 {
            return;
        }
        let mut offset = 0usize;
        while offset < frames {
            let chunk = (frames - offset).min(AUTOMATION_BLOCK_SIZE);
            let block_first = first_frame.saturating_add(offset as u64);
            self.begin_block(block_first);
            let width = chunk * channels;
            let block = &mut out[offset * channels..offset * channels + width];
            for frame in 0..chunk {
                let gain = self.smoother.process();
                if gain == 1.0 {
                    continue;
                }
                for channel in 0..channels {
                    block[frame * channels + channel] *= gain;
                }
            }
            offset += chunk;
        }
    }

    /// 对**主总线**乘“静态主推子 × R55 比率”。
    ///
    /// 主总线没有源：它的静态增益在源侧无法表达，只能在执行之后乘。`base_gain` 是
    /// 调用方算好的静态主增益（`mute` 时传 `0.0`，与 MCP 的既有口径一致）。
    ///
    /// 恒等 ⇒ 只乘 `base_gain`（`1.0` 时一个样本都不碰，调用方也可以直接跳过）。
    pub fn apply_gain_block(
        &mut self,
        out: &mut [f32],
        first_frame: u64,
        frames: usize,
        channels: usize,
        base_gain: f32,
    ) {
        if channels == 0 || frames == 0 {
            return;
        }
        let frames = frames.min(out.len() / channels);
        if self.identity {
            if base_gain == 1.0 {
                return;
            }
            for sample in &mut out[..frames * channels] {
                *sample *= base_gain;
            }
            return;
        }
        let mut offset = 0usize;
        while offset < frames {
            let chunk = (frames - offset).min(AUTOMATION_BLOCK_SIZE);
            let block_first = first_frame.saturating_add(offset as u64);
            self.begin_block(block_first);
            let width = chunk * channels;
            let block = &mut out[offset * channels..offset * channels + width];
            for frame in 0..chunk {
                let gain = base_gain * self.smoother.process();
                if gain == 1.0 {
                    continue;
                }
                for channel in 0..channels {
                    block[frame * channels + channel] *= gain;
                }
            }
            offset += chunk;
        }
    }

    /// 一块开始：求一次曲线值，把平滑器目标设成对应比率。
    fn begin_block(&mut self, first_frame: u64) {
        self.blocks = self.blocks.saturating_add(1);
        let tick = tick_at_frame(first_frame, self.bpm, self.sample_rate);
        // 唯一入口：目标对账 + `read_enabled` 闸 + 曲线插值都在模型层；
        // `#297` 的宏级联在同一处叠加（⛔ 没有第二份插值）。
        let target = match self.composed_db_at(tick) {
            Some(value) => {
                self.last_db = Some(value);
                automation_ratio(value, self.static_gain)
            }
            // 没有自动化值（也没有宏级联）⇒ 退回静态推子（比率 1.0）。
            None => 1.0,
        };
        self.smoother.set_target(target);
    }
}

/// 一条**发送边**的增益自动化（`AutomationTarget::SendGain`）。
///
/// ## 与 `TrackVolume` 的关系
///
/// 求值、5 ms 平滑、恒等快路径**全部**复用 [`TrackGainAutomation`]（同一个
/// `automation_value_at`、同一个 [`ParamSmoother`]、同一套“曲线处处等于静态值”判定）。
/// 本类型只多两件事：
///
/// 1. **静态值来自路由边**（`RoutingEdge::gain_db`，`None` = 单位增益 = 0 dB），
///    不是 `TrackV3::volume_db`；
/// 2. **落点在入边归约**（[`crate::render::RenderPlan`] 的 `render_node`）：把
///    “静态边增益 × R55 比率”逐样本加到目标节点缓冲，而不是乘在一个源的输出上。
///
/// ## 分母
///
/// 与 `TrackVolume` 同一条 R55 纪律：分母是**静态边增益**，⛔ 不是静音/独奏之后的
/// 增益。发送的源若被静音，源输出已经是 0，乘任何比率仍是 0 —— 不会被“复活”。
#[derive(Clone, Debug)]
pub struct SendGainAutomation {
    /// 共用核心（目标永远是 `SendGain`）。
    inner: TrackGainAutomation,
    /// 路由边身份（`inner.target` 的冗余只读副本，方便调用方与计划对账）。
    edge_id: EntityId,
}

impl SendGainAutomation {
    /// 构造一条发送边的求值器。
    ///
    /// 没有泳道 / `read_enabled == false` / 空泳道 / 每个点的线性增益都等于静态边增益
    /// ⇒ [`Self::is_identity`] 为真（调用方走原来的静态累加，一个样本都不多碰）。
    #[must_use]
    pub fn new(
        project: &YebanProjectV1,
        track_id: EntityId,
        edge_id: EntityId,
        sample_rate: u32,
    ) -> Self {
        Self {
            inner: TrackGainAutomation::with_target(
                project,
                AutomationTarget::SendGain { track_id, edge_id },
                track_id,
                sample_rate,
            ),
            edge_id,
        }
    }

    /// 这条自动化所属的路由边身份。
    #[must_use]
    pub const fn edge_id(&self) -> EntityId {
        self.edge_id
    }

    /// 曲线是否**处处**等于静态边增益（⇒ 一个样本都不用碰）。
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// 泳道是否有可求值点（有点且 `read_enabled`）。
    #[must_use]
    pub const fn is_evaluable(&self) -> bool {
        self.inner.is_evaluable()
    }

    /// 泳道的点数。
    #[must_use]
    pub const fn points(&self) -> usize {
        self.inner.points()
    }

    /// 静态边增益的 dB 值。
    #[must_use]
    pub const fn static_db(&self) -> f32 {
        self.inner.static_db()
    }

    /// 静态边增益的线性值。
    #[must_use]
    pub const fn static_gain(&self) -> f32 {
        self.inner.static_gain()
    }

    /// 已被求值的块数（只统计**非恒等**路径）。
    #[must_use]
    pub const fn evaluated_blocks(&self) -> u64 {
        self.inner.evaluated_blocks()
    }

    /// 最近一次块级曲线值（dB）；恒等路径不更新。
    #[must_use]
    pub const fn last_db(&self) -> Option<f32> {
        self.inner.last_db()
    }

    /// 复位到“全新构造”的状态（同一个 [`crate::render::RenderPlan`] 连续执行时调用）。
    pub fn reset_for_execution(&mut self) {
        self.inner.reset_for_execution();
    }

    /// 把 `src` 按“静态边增益 × R55 比率”累加到 `dst`。
    ///
    /// 逐样本平滑（块级求值 + [`ParamSmoother`]）与 [`TrackGainAutomation::apply_gain_block`]
    /// 同一个形状；区别只是这里是 `dst += src * gain`，不是 `dst *= gain`。
    ///
    /// 恒等 ⇒ 走与没有自动化**逐位相同**的 [`crate::sum::accumulate_into`]。
    pub fn accumulate_block(
        &mut self,
        src: &[f32],
        dst: &mut [f32],
        first_frame: u64,
        frames: usize,
        channels: usize,
        base_gain: f32,
    ) {
        if channels == 0 || frames == 0 {
            return;
        }
        let frames = frames.min(src.len() / channels).min(dst.len() / channels);
        if frames == 0 {
            return;
        }
        if self.inner.identity {
            crate::sum::accumulate_into(
                &src[..frames * channels],
                &mut dst[..frames * channels],
                base_gain,
            );
            return;
        }
        let mut offset = 0usize;
        while offset < frames {
            let chunk = (frames - offset).min(AUTOMATION_BLOCK_SIZE);
            let block_first = first_frame.saturating_add(offset as u64);
            self.inner.begin_block(block_first);
            for frame in 0..chunk {
                let gain = base_gain * self.inner.smoother.process();
                for channel in 0..channels {
                    let index = offset * channels + frame * channels + channel;
                    dst[index] += src[index] * gain;
                }
            }
            offset += chunk;
        }
    }
}

/// **声相位置的恒等规则**：`pan == 0`（或非有限）⇒ `(1.0, 1.0)`，否则走调用方给的曲线。
///
/// 这条规则是静声相与声相自动化的**同一个**入口（`#255` 在两侧统一的就是它）：
/// - `crates/yeban-app/src/audio_clip.rs::track_pan_gains`（CLI）
/// - `crates/yeban-mcp/src/domain/render.rs::track_pan_gains`（MCP）
///
/// 都调本函数；于是 `pan == 0` 的“居中是**不衰减**”与 `pan != 0` 的等功率曲线
/// **只有一份实现**。⚠ 自动化曲线在中心附近同样按这条规则取值 ⇒ 同一 `pan` 值下，
/// 自动化给出的左右增益与静态轨逐位相同（乘法的结合次序可能差 ≤ 1 LSB）。
#[must_use]
pub fn centered_pan_gains<F>(pan: f32, curve: F) -> (f32, f32)
where
    F: Fn(f32) -> (f32, f32),
{
    if !pan.is_finite() || pan == 0.0 {
        (1.0, 1.0)
    } else {
        curve(pan)
    }
}

/// 一条音轨的**声相**自动化（`AutomationTarget::TrackPan`）。
///
/// ## 与 [`TrackGainAutomation`] 的关系
///
/// 求值入口、块大小（[`AUTOMATION_BLOCK_SIZE`] = 128 帧）、[`ParamSmoother`]（5 ms）
/// 与恒等快路径**同一套形状**；区别只有三点：
///
/// 1. **值域是声相位置**（`-1.0 ..= 1.0`），不是 dB ⇒ 没有 R55 的“比率”换域，
///    平滑器直接平滑**位置**；
/// 2. **每帧乘的是两个不同的增益**（左 / 右），曲线由调用方以闭包给出
///    （CLI 走 `yeban_engine::mixer::pan_gains`，MCP 走 `render::pan_gains`）——
///    本模块 ⛔ 不复制 `cos`/`sin`，也 ⛔ 不新增依赖；
/// 3. **落点是绝对增益**：调用方必须在非恒等时把源侧静态声相关掉（见模块文档末条），
///    本类型只负责“曲线值 → 平滑 → 左右乘子”。
///
/// ## 恒等快路径
///
/// 没有泳道、`read_enabled == false`、泳道没有点，或者**每个点的值都与静态 `pan` 逐位
/// 相同** ⇒ [`Self::is_identity`] 为真，调用方保持源侧静态声相、整段跳过。
#[derive(Clone, Debug)]
pub struct TrackPanAutomation {
    /// 被自动化的目标（永远是 `TrackPan`）。
    target: AutomationTarget,
    /// 静态声相位置（`TrackV3::pan`）。
    static_pan: f32,
    /// 曲线是否**处处**等于静态位置 ⇒ 恒等快路径。
    identity: bool,
    /// 只含本轨的最小求值工程（与 [`TrackGainAutomation`] 同一手法）。
    evaluator: YebanProjectV1,
    /// 5 ms 一阶低通（平滑的是**声相位置**，不是增益）。
    smoother: ParamSmoother,
    /// 工程速度（tick ↔ 帧）。
    bpm: f64,
    /// 采样率。
    sample_rate: u32,
    /// 泳道是否可求值（有点且 `read_enabled`）。
    evaluable: bool,
    /// 泳道的点数（读数用）。
    points: usize,
    /// 映射到本目标的宏级联（`#297`；空 = 没有 `Macro` 泳道映射到它）。
    macro_cascade: MacroCascade,
    /// 已被求值的块数（读数用）。
    blocks: u64,
    /// 最近一次块级曲线值（声相位置）。
    last_pan: Option<f32>,
}

impl TrackPanAutomation {
    /// 构造一条轨的声相自动化求值器。
    ///
    /// 没有泳道 / `read_enabled == false` / 空泳道 / 每个点的值都等于静态 `pan`
    /// ⇒ [`Self::is_identity`] 为真（调用方应保持静态声相、整段跳过）。
    #[must_use]
    pub fn new(project: &YebanProjectV1, track_id: EntityId, sample_rate: u32) -> Self {
        let target = AutomationTarget::TrackPan { track_id };
        let static_pan = project.tracks.get(&track_id).map_or(0.0, |track| track.pan);
        let lane = project.automation_lane(&target);
        let macro_cascade = MacroCascade::for_target(project, &target);
        let (evaluable, points, lane_identity) = match lane {
            None => (false, 0, true),
            Some(lane) if !lane.read_enabled => (false, lane.points.len(), true),
            Some(lane) if lane.points.is_empty() => (false, 0, true),
            Some(lane) => {
                // 与 `TrackGainAutomation` 同一条充要条件：分段插值在两端相等时整段恒定，
                // 边界是保持 ⇒ 所有点逐位相等就等于整条曲线处处相等。
                let identity = lane
                    .points
                    .values()
                    .all(|point| point.value.to_bits() == static_pan.to_bits());
                (true, lane.points.len(), identity)
            }
        };
        // `#297`：宏级联是独立来源；全部宏点为 0 ⇒ 偏移恒 0，仍走恒等快路径。
        let identity = lane_identity && macro_cascade.is_zero();
        let evaluable = evaluable || macro_cascade.has_sources();
        let points = points + macro_cascade.points();
        // 最小求值工程：只放本轨（`automation_value_at` 的目标对账只看 `tracks`）。
        let mut evaluator = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            pan: static_pan,
            ..TrackV3::default()
        };
        if let Some(track_ref) = project.tracks.get(&track_id) {
            track.automation_lanes = track_ref.automation_lanes.clone();
        }
        evaluator.tracks.insert(track_id, track);
        #[allow(clippy::cast_precision_loss)]
        let smoother = ParamSmoother::with_default_time(sample_rate as f32);
        let mut automation = Self {
            target,
            static_pan,
            identity,
            evaluator,
            smoother,
            bpm: project.bpm,
            sample_rate,
            evaluable,
            points,
            macro_cascade,
            blocks: 0,
            last_pan: None,
        };
        automation.snap_to_start();
        automation
    }

    /// 把平滑器直接置于 tick 0 的曲线值（构造与复位共用）。
    ///
    /// 非有限的静态/曲线值退回**居中**（`0.0`），保持平滑器状态干净。
    fn snap_to_start(&mut self) {
        if self.identity {
            return;
        }
        let start = self.composed_pan_at(0).unwrap_or(self.static_pan);
        self.smoother
            .snap_to(if start.is_finite() { start } else { 0.0 });
    }

    /// 本目标在 `tick` 处的**有效声相位置**：目标泳道（若有）＋ 宏级联偏移。
    ///
    /// 宏级联存在时把结果钳到 `-1.0..=1.0`（位置值域）；没有宏级联时与接线前
    /// 逐位相同（⛔ 不因为本单去钳一条既有泳道）。
    fn composed_pan_at(&self, tick: u64) -> Option<f32> {
        let lane_value = match self.evaluator.automation_value_at(&self.target, tick) {
            Ok(Some(value)) if value.is_finite() => Some(value),
            _ => None,
        };
        let offset = self.macro_cascade.offset_at(tick);
        let value = match lane_value {
            Some(value) => value + offset,
            None if self.macro_cascade.has_sources() => self.static_pan + offset,
            None => return None,
        };
        if !value.is_finite() {
            return None;
        }
        if self.macro_cascade.has_sources() {
            Some(value.clamp(-1.0, 1.0))
        } else {
            Some(value)
        }
    }

    /// 复位到“全新构造”的状态（同一个计划连续执行时调用）。
    pub fn reset_for_execution(&mut self) {
        self.blocks = 0;
        self.last_pan = None;
        self.snap_to_start();
    }

    /// 曲线是否**处处**等于静态声相（⇒ 调用方应保持源侧静态声相并整段跳过）。
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.identity
    }

    /// 泳道是否有可求值点（有点且 `read_enabled`）。
    #[must_use]
    pub const fn is_evaluable(&self) -> bool {
        self.evaluable
    }

    /// 泳道的点数。
    #[must_use]
    pub const fn points(&self) -> usize {
        self.points
    }

    /// 静态声相位置。
    #[must_use]
    pub const fn static_pan(&self) -> f32 {
        self.static_pan
    }

    /// 已被求值的块数（只统计**非恒等**路径）。
    #[must_use]
    pub const fn evaluated_blocks(&self) -> u64 {
        self.blocks
    }

    /// 最近一次块级曲线值（声相位置）；恒等路径不更新。
    #[must_use]
    pub const fn last_pan(&self) -> Option<f32> {
        self.last_pan
    }

    /// 对**源输出**乘自动化声相的左右增益（绝对增益，不是比率）。
    ///
    /// 调用方前置条件：非恒等时源侧静态声相已被关掉（`pan = 0`），否则乘两次。
    ///
    /// `curve` 是该侧的**唯一**等功率曲线（`pan → (左, 右)`）；中心（`pan == 0`）
    /// 的“不衰减”由 [`centered_pan_gains`] 统一处理。恒等 ⇒ **整段跳过**。
    ///
    /// 声道口径与源侧静态声相**同一形状**：第 0 声道左、第 1 声道右，其余声道在
    /// 非中心时保持 0（源侧静态声相就是只写这两路）。
    pub fn apply_pan_block<F>(
        &mut self,
        out: &mut [f32],
        first_frame: u64,
        frames: usize,
        channels: usize,
        curve: F,
    ) where
        F: Fn(f32) -> (f32, f32),
    {
        if self.identity || channels == 0 {
            return;
        }
        let frames = frames.min(out.len() / channels);
        if frames == 0 {
            return;
        }
        let mut offset = 0usize;
        while offset < frames {
            let chunk = (frames - offset).min(AUTOMATION_BLOCK_SIZE);
            let block_first = first_frame.saturating_add(offset as u64);
            self.begin_block(block_first);
            let block = &mut out[offset * channels..(offset + chunk) * channels];
            for frame in 0..chunk {
                let pan = self.smoother.process();
                let (left, right) = centered_pan_gains(pan, &curve);
                if left == 1.0 && right == 1.0 {
                    continue;
                }
                let base = frame * channels;
                block[base] *= left;
                if channels > 1 {
                    block[base + 1] *= right;
                }
                // 与源侧静态声相同口径：非中心时其余声道不参与。
                for channel in 2..channels {
                    block[base + channel] = 0.0;
                }
            }
            offset += chunk;
        }
    }

    /// 一块开始：求一次曲线值，把平滑器目标设成该声相位置。
    fn begin_block(&mut self, first_frame: u64) {
        self.blocks = self.blocks.saturating_add(1);
        let tick = tick_at_frame(first_frame, self.bpm, self.sample_rate);
        // 唯一入口：目标对账 + `read_enabled` 闸 + 曲线插值都在模型层；
        // `#297` 的宏级联在同一处叠加。
        let pan = match self.composed_pan_at(tick) {
            Some(value) => {
                self.last_pan = Some(value);
                value
            }
            // 此刻没有自动化值（也没有宏级联）⇒ 退回静态声相（源侧已被关掉，
            // 所以这里必须是静态位置，而不是居中）。
            None => self.static_pan,
        };
        self.smoother.set_target(pan);
    }
}

/// 本切片**真的会落点**的设备参数名（**含别名**，`41` 个拼写 = `22` 个字段）。
///
/// `#266` 只开 `output_gain_db`；`#267` 扩到**通道条那一整组**（27 拼写 / 14 字段）；
/// `#293` 再扩到 **Freeverb 的 4 个标量字段**（6 拼写）与**卷积混响的 4 个标量字段**
/// （8 拼写）。
///
/// ⛔ 不是"白名单"：`crates/yeban-render/src/inserts.rs::is_supported_param` 的名字是
/// **静态投影**消费的全集（多三个构造期 IR 形状名）；本表是**自动化**这一层接上的
/// 子集。判别处**只有一处** —— [`crate::inserts::DeviceParamField::from_name`]；
/// 本表是它的可读清单，两者由 `the_applied_names_and_the_field_table_agree` 双向钉住
/// （⛔ 不许只改一边）。
///
/// ## 为什么**是**这些
///
/// - 27 个通道条拼写对应 [`crate::inserts::StripField`] 的 14 个字段；
/// - 6 个混响拼写对应 [`crate::inserts::ReverbField`] 的 4 个字段
///   （`reverb_size` / `reverb_damp` / `reverb_wet`(＋`_mix`) / `reverb_predelay`(＋`_s`)）；
/// - 8 个卷积拼写对应 [`crate::inserts::ConvolutionField`] 的 4 个字段
///   （`conv_predelay*` / `conv_dry` / `conv_wet`(＋`_mix`) / `conv_ir_gain*`）。
///
/// 三者与静态投影**同一张拼写表** ⇒ 一条泳道改的正是静态投影会交给对应器件
/// `set_params` 的那个字段。
///
/// ## 为什么**不**是别的名字
///
/// - **压缩**（`threshold_db` / `ratio` / `knee_db` / `detector_s` / `attack_s` /
///   `release_s` / `makeup_db` / `compressor_enabled`）：离线侧**整级不执行**
///   （`inserts.rs` 模块头 §1：检波器在声相前后的电平口径未裁决）⇒ 改它一个字节都
///   不会变。接上就是假话；
/// - **卷积 IR 形状**（`conv_ir_decay_s` / `conv_ir_decay` / `conv_ir_seed`）：那是
///   **构造期烘焙**，换值要重新合成 IR 并走 `set_impulse_response`（分配入口）⇒
///   每 128 帧重烘会破坏“IR 构造期烘好 / `process` 零分配”的架构契约；
/// - 未知旋钮。
pub const APPLIED_DEVICE_PARAM_NAMES: &[&str] = &[
    // 通道条（27 = 14 个字段，含别名）。
    "input_gain_db",
    "input_gain",
    "trim_db",
    "output_gain_db",
    "output_gain",
    "fader_db",
    "eq_enabled",
    "filter_enabled",
    "eq_low_gain",
    "low_gain",
    "eq_low_freq",
    "low_freq",
    "eq_mid_gain",
    "mid_gain",
    "eq_mid_freq",
    "mid_freq",
    "eq_mid_q",
    "mid_q",
    "eq_high_gain",
    "high_gain",
    "eq_high_freq",
    "high_freq",
    "cutoff_hz",
    "resonance",
    "filter_resonance",
    "drive",
    "filter_drive",
    // Freeverb 标量（6 = 4 个字段）。
    "reverb_size",
    "reverb_damp",
    "reverb_wet",
    "reverb_wet_mix",
    "reverb_predelay",
    "reverb_predelay_s",
    // 卷积混响标量（8 = 4 个字段；⛔ 不含 `conv_ir_decay*` / `conv_ir_seed`）。
    "conv_predelay_s",
    "conv_pre_delay_s",
    "conv_predelay",
    "conv_dry",
    "conv_wet",
    "conv_wet_mix",
    "conv_ir_gain_db",
    "conv_ir_gain",
];

/// 名字是否是本切片会落点的设备参数（**大小写不敏感**，与静态投影的同一口径）。
///
/// 唯一判定处是 [`crate::inserts::DeviceParamField::from_name`]（本表只是它的可读清单）。
#[must_use]
pub fn is_applied_device_param_name(name: &str) -> bool {
    DeviceParamField::from_name(name).is_some()
}

/// 本轨通道条的**静态基值**：投影来源的参数；没有来源时是
/// [`forced_identity_strip`]（`#266` 的缝，`#267` 原样沿用）。
///
/// 为什么必须是**这一份**而不是 `ChannelStripParams::DEFAULT`：强制实例化时器件拿到的
/// 就是 `effective_strip()` 的这一个分支（三级全关），它的 `eq_enabled` / `filter_enabled`
/// 与 `DEFAULT`（三级全开）**不同** —— 恒等判定若比 `DEFAULT`，一条 `eq_enabled: 0`
/// 的泳道会被误判成"非恒等"，于是白实例化一台器件。
fn static_strip(track: &TrackV3) -> ChannelStripParams {
    strip_source_params(track).unwrap_or_else(forced_identity_strip)
}

/// 本轨某个**插入器件族**的静态字段值（恒等判定用）。
///
/// - 通道条沿用 [`static_strip`] 的 `#266` 缝（全恒等增益设备也是来源）；
/// - Freeverb / 卷积没有那条缝：目标参数名本身就是 `reverb_of` / `convolution_of`
///   的“已识别”证据 ⇒ 来源设备必然存在，`None` 只是“不是来源”的报错。
///
/// 混响/卷积的静态值在这里先过 [`DeviceParamField::sanitise`]（与器件 `set_params`
/// 同一次钳制），因此恒等判定比的是**器件读数**，不是原始旋钮值。
fn static_insert_field_value(track: &TrackV3, field: DeviceParamField) -> Option<f32> {
    match field {
        DeviceParamField::Strip(field) => Some(field.read(&static_strip(track))),
        DeviceParamField::Reverb(field) => {
            let params = reverb_source_params(track)?;
            Some(field.sanitise(field.read(&params)))
        }
        DeviceParamField::Convolution(field) => {
            let params = convolution_source_params(track)?;
            Some(field.sanitise(field.read(&params)))
        }
    }
}

/// 把一条 `DeviceParam` 泳道**解析成落点**：
/// `(参数名原文, 设备身份, 落点字段, 该字段的静态值)`。
///
/// 返回 `None` 的情形（全部是"不落点"，⛔ 不是沉默）：
///
/// 1. 序数地址解析失败（轨 / 插槽 / 参数下标越界）；
/// 2. 参数名不是任何器件族的字段（[`crate::inserts::DeviceParamField::from_name`]）；
/// 3. 设备被旁通 / 不是内置效果器（规则同 `from_devices`）；
/// 4. 该插槽**不是**本轨对应器件族的来源设备（`strip_source_slot` /
///    `reverb_source_slot` / `convolution_source_slot`）—— 那台设备的参数根本不会
///    进 DSP，改它没有落点。
///
/// ## "全恒等设备"也是来源（`#266` 的边界裁决，`#267` 逐条沿用）
///
/// [`crate::inserts::InsertChain::from_devices`] 的"已识别"规则有一条兜底：
/// `params != DEFAULT`（与实时引擎 `InsertParams::from_devices` 的 `into_params`
/// 逐字同式）。于是**只写 `output_gain_db: 0.0`**（或 `input_gain_db: 0.0` —— 两个增益
/// 是唯一"写默认值也不置 `seen` 位"的字段）的设备投影成 `None` —— 它在引擎侧
/// **不武装**通道条。但它的静态投影**恰好就是恒等**，所以一条泳道指向它时，
/// "强制实例化一台 [`forced_identity_strip`] 参数的通道条"就是它的忠实静态基值。
/// 本函数因此在这一个缝里承认它是来源：**仅当整条轨没有任何其它通道条来源**时
/// （有来源 ⇒ 只认那台，⛔ 不把参数改到另一台设备上）。
///
/// ⚠ 缝只对**增益**字段存在：`eq_*` / `cutoff_hz` / `resonance` / `drive` / 两个开关
/// 只要出现就把 `StripProjection` 的 `seen` 位置起 ⇒ `strip_of` 返回 `Some`，走的是
/// 正常来源分支（静态值仍是投影出来的那一个）。因此"设备改不动"的情形不会因为
/// 扩名字而出现。混响/卷积**没有**这条缝：任何已识别名字都让对应 `*_of` 返回 `Some`。
///
/// 这是**唯一**的判定处：`DeviceParamAutomation::new` 与 MCP 的 `AutomationReport`
/// 都调它 ⇒ "报告说应用了"与"真的挂上了"不会各判一套。
fn applicable_device_param(
    project: &YebanProjectV1,
    track_id: EntityId,
    slot_index: usize,
    param_index: usize,
) -> Option<(String, EntityId, DeviceParamField, f32)> {
    let resolved = resolve_device_param(project, track_id, slot_index, param_index).ok()?;
    let field = DeviceParamField::from_name(&resolved.param_name)?;
    let track = project.tracks.get(&track_id)?;
    let slot_is_source = match field.family() {
        DeviceParamFamily::Strip => match strip_source_slot(track) {
            Some(source) => source == slot_index,
            None => track.devices.get(slot_index).is_some_and(|device| {
                !device.bypassed && device.kind == yeban_model::DeviceKind::InternalEffect
            }),
        },
        DeviceParamFamily::Reverb => reverb_source_slot(track) == Some(slot_index),
        DeviceParamFamily::Convolution => convolution_source_slot(track) == Some(slot_index),
    };
    if !slot_is_source {
        return None;
    }
    // 静态值 = **器件实际拿到的**投影参数里这个字段的值（混响/卷积已按器件同一次
    // 钳制算过；通道条投影本身已 `sanitised`，缝里那一份也是全定义）。
    let static_value = static_insert_field_value(track, field)?;
    Some((resolved.param_name, resolved.device_id, field, static_value))
}

/// 一条 `DeviceParam` 泳道是否**会被**接上（MCP 的 `AutomationReport` 用）。
///
/// 与 [`DeviceParamAutomation::new`] 的 `applicable` 位**同一处判定**
/// （[`applicable_device_param`]），⛔ 不写第二套规则。
#[must_use]
pub fn device_param_lane_is_applicable(
    project: &YebanProjectV1,
    target: &AutomationTarget,
) -> bool {
    match target {
        AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        } => applicable_device_param(project, *track_id, *slot_index, *param_index).is_some(),
        _ => false,
    }
}

/// 一条自动化目标在**离线渲染**里**没有落点**的原因。
///
/// 这是 [`unsupported_lane_reason`] 的返回值，也是 `yeban_edit_automation` 读侧
/// `residualReason` 的**唯一**字面量来源（⛔ 不在 MCP 侧再拼一份）。
///
/// ⚠ 名字 `BusTrack` 的口径是"**该轨在渲染计划里没有落点**"：它现在**只**覆盖
/// "不在保留子图里的不可达轨（`TrackVolume` / `TrackPan` 都没有落点）"。B2 起
/// **可达的总线轨**也有 `TrackPan` 落点（输出级的静态声相 / 自动化），不再报它。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneResidual {
    /// `Macro`：宏泳道**没有可落点**（`#297`）。
    ///
    /// 落点 = 宏存在 ∧ 至少一条 mapping ∧ **每条** mapping 的目标都落点（递归判定；
    /// 目标是 `Macro` 或成环、空 mappings 都仍报本残差）。
    Macro,
    /// `DeviceParam`：没有插入链落点（名字不是通道条 / Freeverb / 卷积标量字段、
    /// 设备被旁通、或指向的不是该器件族的来源设备）。
    ///
    /// ⚠ 名字里的 `NotChannelStrip` 是**外部契约的稳定串**，本单 `#293` 扩到
    /// Freeverb/卷积后**不改**：它仍是“离线插入链没有这个落点”的唯一残差值。
    DeviceParamNotChannelStrip,
    /// 目标轨**没有落点**（**只有不可达轨**）：
    /// - `TrackVolume`：不在渲染计划里。计划内的总线轨与源轨**都有**落点
    ///   （总线轨是 `#296` 新增的输出级推子；源轨是源侧静态推子）；
    /// - `TrackPan`：不在渲染计划里。计划内的源轨走 `#257` 的源侧门，**总线轨**走
    ///   B2 的输出级声相（静态与自动化同一处落点）。
    BusTrack,
    /// `SendGain` 的路由边不在**保留子图**里（被剪枝）。
    PrunedEdge,
}

impl LaneResidual {
    /// 响应里的稳定名字（`yeban_edit_automation` 的 `residualReason`）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macro => "macro",
            Self::DeviceParamNotChannelStrip => "deviceParamNotChannelStrip",
            Self::BusTrack => "busTrack",
            Self::PrunedEdge => "prunedEdge",
        }
    }
}

/// 离线渲染对一条自动化目标**有没有落点**的**唯一判定**（`None` = 有落点）。
///
/// 三个切片是**同一次**计划探针的三个视图（`RenderPlan::nodes()`、其中的源节点、
/// `RenderPlan::edge_ids()`），因此本函数与 `yeban_render_master` 的
/// `AutomationReport`（`data.automation[].supported`）是**同一份事实**：
///
/// - `TrackVolume`：主总线，或**计划里的任何节点**（`#296`；源节点在源侧乘、总线节点
///   在新增的输出级乘；只有**不可达轨**仍为 `busTrack`）；
/// - `SendGain`：边在保留子图里（`#256`）；
/// - `TrackPan`：主总线（`#296` 起有声相落点）、计划里的源节点（`#257`）或
///   **计划里的总线轨**（B2：输出级声相；只有不可达轨仍是 `busTrack`）；
/// - `DeviceParam`：节点在计划里且参数名有**插入链落点**（通道条 / Freeverb 标量 /
///   卷积标量；`#266`/`#267`/`#293`）；
/// - `Macro`：宏存在、至少一条 mapping、且每条 mapping 的目标本身有落点
///   （`#297`；目标是 `Macro` / 成环 / 空 mappings ⇒ 仍报 `macro`）。
///
/// ⛔ 这里是判定落点的**唯一处**：`yeban-mcp` 的 `AutomationReport` 与
/// `yeban_edit_automation` 的读侧都调它 —— 第二份 match 就是第二份会漂移的事实。
#[must_use]
pub fn unsupported_lane_reason(
    project: &YebanProjectV1,
    target: &AutomationTarget,
    source_nodes: &[EntityId],
    rendered_edges: &[EntityId],
    rendered_nodes: &[EntityId],
) -> Option<LaneResidual> {
    match target {
        AutomationTarget::TrackVolume { track_id } => {
            // ⭐ `#296`：计划里的**总线轨**也有落点了（`render_node` 的输出级推子）。
            // 因此判据从"有源落点"改成"在保留子图里"：不可达轨仍如实报 `busTrack`。
            if *track_id == project.master_bus_track_id || rendered_nodes.contains(track_id) {
                None
            } else {
                Some(LaneResidual::BusTrack)
            }
        }
        AutomationTarget::SendGain { edge_id, .. } => {
            if rendered_edges.contains(edge_id) {
                None
            } else {
                Some(LaneResidual::PrunedEdge)
            }
        }
        AutomationTarget::TrackPan { track_id } => {
            // ⭐ B2：与 `TrackVolume` 同一条判据 —— 主总线，或**保留子图里的任何节点**。
            // 源节点走 `#257` 的源侧门；总线节点走 B2 的输出级声相（`render_node`）；
            // 只有不可达轨（不在 `rendered_nodes`）仍如实报 `busTrack`。
            if *track_id == project.master_bus_track_id || rendered_nodes.contains(track_id) {
                None
            } else {
                Some(LaneResidual::BusTrack)
            }
        }
        AutomationTarget::DeviceParam { track_id, .. } => {
            if rendered_nodes.contains(track_id) && device_param_lane_is_applicable(project, target)
            {
                None
            } else {
                Some(LaneResidual::DeviceParamNotChannelStrip)
            }
        }
        AutomationTarget::Macro {
            track_id,
            macro_index,
        } => {
            if macro_lane_lands(
                project,
                *track_id,
                *macro_index,
                source_nodes,
                rendered_edges,
                rendered_nodes,
            ) {
                None
            } else {
                Some(LaneResidual::Macro)
            }
        }
    }
}

/// 一条 `Macro` 泳道是否**真的会落点**（任务单 `#297` 的唯一判定）。
///
/// 条件（裁决逐条）：宏存在 ∧ 至少一条 mapping ∧ 每条 mapping 的目标都落点。
/// - 目标本身是 `Macro` ⇒ `false`（嵌套宏本切片不消费；也正因如此，
///   这个判定**不会递归进宏链**，自指/成环不会栈溢出）；
/// - `visiting` 仍显式记录已经走过的目标：即便将来改成递归消费嵌套宏，
///   环会被在这里挡住（⛔ 不是靠调用深度兜底）。
fn macro_lane_lands(
    project: &YebanProjectV1,
    track_id: EntityId,
    macro_index: usize,
    source_nodes: &[EntityId],
    rendered_edges: &[EntityId],
    rendered_nodes: &[EntityId],
) -> bool {
    let Some(track) = project.tracks.get(&track_id) else {
        return false;
    };
    let Some(macro_parameter) = track.macros.get(macro_index) else {
        return false;
    };
    if macro_parameter.mappings.is_empty() {
        return false;
    }
    let mut visiting = BTreeSet::new();
    visiting.insert(AutomationTarget::Macro {
        track_id,
        macro_index,
    });
    macro_parameter.mappings.iter().all(|mapping| {
        let target = mapping.target;
        if !visiting.insert(target) {
            return false;
        }
        let lands = match target {
            AutomationTarget::Macro { .. } => false,
            _ => unsupported_lane_reason(
                project,
                &target,
                source_nodes,
                rendered_edges,
                rendered_nodes,
            )
            .is_none(),
        };
        visiting.remove(&target);
        lands
    })
}

/// 一条音轨上的**设备参数**自动化（`AutomationTarget::DeviceParam`）。
///
/// ## 与增益 / 声相自动化的形状差别（任务单 `#262` 逐条，本类型必须照它）
///
/// `TrackVolume` / `SendGain` 是"**逐样本乘一个标量**"，所以它们走
/// [`ParamSmoother`]（5 ms）；`TrackPan` 平滑的是位置、乘的是两个绝对增益。而
/// `DeviceParam` 改的是**一台器件的参数**，器件换参数就要**重算系数**
/// （[`crate::inserts::InsertRuntime::apply_device_params`] 走对应器件的 `set_params`）
/// ⇒ 只能在**块边界**做一次，⛔ 不能逐样本线性插值"效果"。本类型因此：
///
/// - 每 [`AUTOMATION_BLOCK_SIZE`]（128）帧求值一次，把该字段的**原生绝对值**交给运行态
///   （字段由 [`Self::field`] 给出：`#267` 起是通道条整组，`#293` 起含 Freeverb /
///   卷积标量；⛔ 不再只有输出增益）；
/// - **不持有** `ParamSmoother`，也**不**乘任何样本；
/// - 恒等（没有落点 / 没有泳道 / `read_enabled:false` / 空泳道 / 每个点的
///   **钳制后**值都等于静态值）⇒ 调用方一个器件都不强制实例化。
///
/// ## 单位
///
/// **原生绝对值**，与 `TrackPan`／`SendGain` 同一条先例：⛔ 没有 0..1
/// 归一化，也 ⛔ 没有 R55 换域 —— 交出去的数直接就是器件参数（`_db` = dB、
/// `_hz` = Hz、秒 = 秒、开关 `0.0`/`1.0`）。
///
/// ## 为什么求值器仍是一份"只含本轨的模型工程"
///
/// 与 [`TrackGainAutomation`] 同一手法：泳道与**设备链**（目标对账要按下标取设备）
/// 在构造期克隆，之后只读；每块对同一份 [`YebanProjectV1::automation_value_at`] 求值。
#[derive(Clone, Debug)]
pub struct DeviceParamAutomation {
    /// 被自动化的目标（永远是 `DeviceParam`）。
    target: AutomationTarget,
    /// 目标所属音轨（`target.track_id()` 的只读副本）。
    track_id: EntityId,
    /// 是否**有落点**（名字/来源设备/序数都对得上）；`false` ⇒ 恒等。
    applicable: bool,
    /// 参数名**原文**（读数用）。
    param_name: String,
    /// 这条泳道改的是**哪个插入器件字段**（没有落点时为 `None`）。
    field: Option<DeviceParamField>,
    /// 该字段的静态值（与器件拿到的投影参数同源，已按器件钳制；开关是 `0.0`/`1.0`）。
    static_value: f32,
    /// 曲线是否**处处**等于静态值（或本就没有落点）⇒ 恒等快路径。
    identity: bool,
    /// 只含本轨的最小求值工程（含设备链，`validate_against` 要按序数取）。
    evaluator: YebanProjectV1,
    /// 工程速度（tick ↔ 帧）。
    bpm: f64,
    /// 采样率。
    sample_rate: u32,
    /// 泳道是否可求值（有点且 `read_enabled`）。
    evaluable: bool,
    /// 泳道的点数（读数用）。
    points: usize,
    /// 映射到本目标的宏级联（`#297`；空 = 没有 `Macro` 泳道映射到它）。
    macro_cascade: MacroCascade,
    /// 已被求值的块数（只统计非恒等路径）。
    blocks: u64,
    /// 最近一次块级曲线值（原生绝对值；开关是 `0.0`/`1.0`）。
    last_value: Option<f32>,
}

impl DeviceParamAutomation {
    /// 构造一条设备参数自动化求值器。
    ///
    /// 没有落点 / 没有泳道 / `read_enabled == false` / 空泳道 / 每个点的钳制后值都
    /// 等于静态值 ⇒ [`Self::is_identity`] 为真（调用方应整段跳过、且不强制实例化器件）。
    #[must_use]
    pub fn new(
        project: &YebanProjectV1,
        track_id: EntityId,
        slot_index: usize,
        param_index: usize,
        sample_rate: u32,
    ) -> Self {
        let target = AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        };
        let resolved = applicable_device_param(project, track_id, slot_index, param_index);
        let applicable = resolved.is_some();
        let (param_name, field, static_value) = resolved.map_or_else(
            || (String::new(), None, 0.0),
            |(name, _device_id, field, static_value)| (name, Some(field), static_value),
        );
        let lane = project.automation_lane(&target);
        let macro_cascade = MacroCascade::for_target(project, &target);
        let (evaluable, points, curve_identity) = match lane {
            None => (false, 0, true),
            Some(lane) if !lane.read_enabled => (false, lane.points.len(), true),
            Some(lane) if lane.points.is_empty() => (false, 0, true),
            Some(lane) => {
                // 与 `TrackGainAutomation` 同一条充要条件：分段插值在两端相等时整段恒定，
                // 边界是保持 ⇒ 所有点**钳制后**相等就等于整条曲线处处相等。
                // 钳制走 `DeviceParamField::sanitise`（对应器件 `set_params` 内部同一次
                // 钳制；混响/卷积的镜像由真实器件网格判据钉住）⇒ 恒等判定不会因为
                // "`-100 dB` 与 `-60 dB` 交给器件是同一个值"而漏判。
                let identity = match field {
                    Some(field) => lane.points.values().all(|point| {
                        field.sanitise(point.value).to_bits() == static_value.to_bits()
                    }),
                    None => true,
                };
                (true, lane.points.len(), identity)
            }
        };
        // 最小求值工程：只放本轨，**设备链也克隆**（`validate_against` 要按序数取设备）。
        // 泳道数据原样克隆 ⇒ 求值语义没有第二份。
        let mut evaluator = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            ..TrackV3::default()
        };
        if let Some(track_ref) = project.tracks.get(&track_id) {
            track.automation_lanes = track_ref.automation_lanes.clone();
            track.devices = track_ref.devices.clone();
        }
        evaluator.tracks.insert(track_id, track);
        // `#297`：宏级联是独立来源；全部宏点为 0 ⇒ 偏移恒 0，仍走恒等快路径
        // （不强制实例化器件、一个参数都不改）。
        let identity = !applicable || (curve_identity && macro_cascade.is_zero());
        let evaluable = evaluable || macro_cascade.has_sources();
        let points = points + macro_cascade.points();
        Self {
            target,
            track_id,
            applicable,
            param_name,
            field,
            static_value,
            identity,
            evaluator,
            bpm: project.bpm,
            sample_rate,
            evaluable,
            points,
            macro_cascade,
            blocks: 0,
            last_value: None,
        }
    }

    /// 目标（永远是 `DeviceParam`）。
    #[must_use]
    pub const fn target(&self) -> AutomationTarget {
        self.target
    }

    /// 目标所属音轨。
    #[must_use]
    pub const fn track_id(&self) -> EntityId {
        self.track_id
    }

    /// 是否有落点（名字 ∈ [`APPLIED_DEVICE_PARAM_NAMES`] 且指向对应器件族的来源设备）。
    #[must_use]
    pub const fn is_applicable(&self) -> bool {
        self.applicable
    }

    /// 参数名原文（没有落点时为 `""`）。
    #[must_use]
    pub fn param_name(&self) -> &str {
        &self.param_name
    }

    /// 这条泳道改的**插入器件字段**（没有落点时为 `None`）。
    ///
    /// `value_at_block` 返回 `Some` ⇒ 这里必然是 `Some`（非恒等 ⇒ 有落点）。
    #[must_use]
    pub const fn field(&self) -> Option<DeviceParamField> {
        self.field
    }

    /// 该字段的**静态值**（与器件拿到的投影参数同源；开关是 `0.0` / `1.0`）。
    #[must_use]
    pub const fn static_value(&self) -> f32 {
        self.static_value
    }

    /// 曲线是否**处处**等于静态值（或没有落点）⇒ 一个器件都不用实例化。
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.identity
    }

    /// 泳道是否有可求值点（有点且 `read_enabled`）。
    #[must_use]
    pub const fn is_evaluable(&self) -> bool {
        self.evaluable
    }

    /// 泳道的点数。
    #[must_use]
    pub const fn points(&self) -> usize {
        self.points
    }

    /// 已被求值的块数（只统计非恒等路径）。
    #[must_use]
    pub const fn evaluated_blocks(&self) -> u64 {
        self.blocks
    }

    /// 最近一次块级曲线值（原生绝对值；开关是 `0.0` / `1.0`）；恒等路径不更新。
    #[must_use]
    pub const fn last_value(&self) -> Option<f32> {
        self.last_value
    }

    /// 复位到"全新构造"的状态（同一个 [`crate::render::RenderPlan`] 连续执行时调用）。
    ///
    /// 本类型**没有**跨块时间状态（没有平滑器、器件状态在 [`crate::inserts`] 里），
    /// 因此只清读数；器件的复位由 `RenderPlan::execute` 对 `InsertRuntime::reset` 做。
    pub fn reset_for_execution(&mut self) {
        self.blocks = 0;
        self.last_value = None;
    }

    /// 一块开始：求一次曲线值（**块边界**调用一次，⛔ 不是逐样本）。
    ///
    /// 恒等 ⇒ `None`（调用方不改任何参数）。非恒等但此刻没有曲线值
    /// （`Ok(None)` / `Err(_)`）⇒ 退回静态值（与 `TrackGainAutomation` 同一条兜底）。
    pub fn value_at_block(&mut self, first_frame: u64) -> Option<f32> {
        if self.identity {
            return None;
        }
        self.blocks = self.blocks.saturating_add(1);
        let tick = tick_at_frame(first_frame, self.bpm, self.sample_rate);
        // 唯一入口：目标对账 + `read_enabled` 闸 + 曲线插值都在模型层；
        // `#297` 的宏级联在同一处叠加（目标没有泳道时基值 = 静态值）。
        let mut value = match self.evaluator.automation_value_at(&self.target, tick) {
            Ok(Some(value)) if value.is_finite() => value,
            _ => self.static_value,
        };
        value += self.macro_cascade.offset_at(tick);
        if !value.is_finite() {
            value = self.static_value;
        }
        self.last_value = Some(value);
        Some(value)
    }
}

/// 工程里**被 `Macro` 泳道映射到的目标**（去重；`Macro` 目标本身除外）。
///
/// 只从**存在的 `Macro` 泳道**出发：没有宏泳道的静态宏不参与渲染（否则与
/// `yeban_set_macro` 物化进目标泳道的级联双计）。关掉/空泳道也算"存在"——
/// 它产出的自动化是恒等的，调用方按既有快路径跳过。
fn macro_mapped_targets(project: &YebanProjectV1) -> BTreeSet<AutomationTarget> {
    let mut targets = BTreeSet::new();
    for track in project.tracks.values() {
        for lane_target in track.automation_lanes.keys() {
            let AutomationTarget::Macro {
                track_id,
                macro_index,
            } = lane_target
            else {
                continue;
            };
            let Some(macro_track) = project.tracks.get(track_id) else {
                continue;
            };
            let Some(macro_parameter) = macro_track.macros.get(*macro_index) else {
                continue;
            };
            for mapping in &macro_parameter.mappings {
                if !matches!(mapping.target, AutomationTarget::Macro { .. }) {
                    targets.insert(mapping.target);
                }
            }
        }
    }
    targets
}

/// 扫描工程里**全部** `DeviceParam` 泳道，按轨分组（按轨身份升序）。
///
/// 两个导出（MCP 的 `yeban_render_master` 与 app 的 `--export-wav`）都调它 ⇒
/// "哪些设备参数泳道要被接上"只有一份实现。**没有落点的泳道也会返回**（恒等）：
/// 调用方按 [`DeviceParamAutomation::is_identity`] 决定是否强制实例化器件。
///
/// `#297`：宏泳道映射到的设备参数也会被包含（它可能没有自己的泳道）。
#[must_use]
pub fn device_param_automations(
    project: &YebanProjectV1,
    sample_rate: u32,
) -> BTreeMap<EntityId, Vec<DeviceParamAutomation>> {
    // `#297`：除了既有 `DeviceParam` 泳道，宏泳道映射到的设备参数也要接上
    // （它可能没有自己的泳道）。按 `(轨, 槽, 参数)` 去重，构造顺序与接线前一致。
    let mut wanted: BTreeSet<(EntityId, usize, usize)> = BTreeSet::new();
    for (track_id, track) in &project.tracks {
        for target in track.automation_lanes.keys() {
            if let AutomationTarget::DeviceParam {
                slot_index,
                param_index,
                ..
            } = target
            {
                wanted.insert((*track_id, *slot_index, *param_index));
            }
        }
    }
    for target in macro_mapped_targets(project) {
        if let AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        } = target
        {
            wanted.insert((track_id, slot_index, param_index));
        }
    }
    let mut out: BTreeMap<EntityId, Vec<DeviceParamAutomation>> = BTreeMap::new();
    for (track_id, slot_index, param_index) in wanted {
        out.entry(track_id)
            .or_default()
            .push(DeviceParamAutomation::new(
                project,
                track_id,
                slot_index,
                param_index,
                sample_rate,
            ));
    }
    out
}

/// 需要**按器件族强制实例化**的轨（任务单 `#266` 的通道条 ＋ `#293` 的混响/卷积）。
///
/// 这就是"编译期开关"的数据：`project_inserts_with_forced_instances` 用它让
/// 通道条在"全恒等参数"下也返回 `Some`、让 Freeverb/卷积在 `wet = 0`（`is_active()`
/// 为假）时仍建出运行态 —— 否则块边界没有器件可改，泳道会"报 applied 却静默无效"。
///
/// ⚠ **按族收窄**：只有落点族等于该族的非恒等泳道才置位。混响泳道不会白实例化一条
/// 恒等通道条（`#293` 判据）；恒等泳道一律不置位（否则产物会变）。
#[must_use]
pub fn forced_insert_instances(
    automations: &BTreeMap<EntityId, Vec<DeviceParamAutomation>>,
) -> ForcedInsertInstances {
    let mut forced = ForcedInsertInstances::default();
    for (track_id, list) in automations {
        for automation in list.iter().filter(|automation| !automation.is_identity()) {
            let Some(field) = automation.field() else {
                continue;
            };
            match field.family() {
                DeviceParamFamily::Strip => {
                    forced.strip.insert(*track_id);
                }
                DeviceParamFamily::Reverb => {
                    forced.reverb.insert(*track_id);
                }
                DeviceParamFamily::Convolution => {
                    forced.convolution.insert(*track_id);
                }
            }
        }
    }
    forced
}

/// 需要**强制实例化通道条**的轨（`forced_insert_instances` 的 `strip` 族）。
///
/// 保留这个入口是为了既有调用点/判据的签名不变；它是同一个扫描器的**投影**，
/// ⛔ 不是第二套判定。
#[must_use]
pub fn forced_strip_tracks(
    automations: &BTreeMap<EntityId, Vec<DeviceParamAutomation>>,
) -> BTreeSet<EntityId> {
    forced_insert_instances(automations).strip
}

/// 扫描工程里**全部** `SendGain` 泳道，给每条边构造一个求值器（按边身份升序）。
///
/// 两个导出（MCP 的 `yeban_render_master` 与 app 的 `--export-wav`）都调它 ⇒
/// “哪些泳道要被接上”只有一份实现。恒等泳道也会返回（调用方按
/// [`SendGainAutomation::is_identity`] 走静态路径，结果与“没有泳道”逐字节相同）。
#[must_use]
pub fn send_gain_automations(
    project: &YebanProjectV1,
    sample_rate: u32,
) -> BTreeMap<EntityId, SendGainAutomation> {
    // `#297`：既有 `SendGain` 泳道 ＋ 宏泳道映射到的发送边。
    let mut wanted: BTreeSet<(EntityId, EntityId)> = BTreeSet::new();
    for (track_id, track) in &project.tracks {
        for target in track.automation_lanes.keys() {
            if let AutomationTarget::SendGain { edge_id, .. } = target {
                wanted.insert((*track_id, *edge_id));
            }
        }
    }
    for target in macro_mapped_targets(project) {
        if let AutomationTarget::SendGain { track_id, edge_id } = target {
            wanted.insert((track_id, edge_id));
        }
    }
    let mut out = BTreeMap::new();
    for (track_id, edge_id) in wanted {
        out.insert(
            edge_id,
            SendGainAutomation::new(project, track_id, edge_id, sample_rate),
        );
    }
    out
}

/// 扫描工程里**全部** `TrackPan` 泳道，给每条轨构造一个求值器（按轨身份升序）。
///
/// 两个导出（MCP 的 `yeban_render_master` 与 app 的 `--export-wav`）都走同一个
/// [`TrackPanAutomation`] ⇒ “哪些声相泳道要被接上”只有一份实现。恒等泳道也会返回
/// （调用方保持源侧静态声相，结果与“没有泳道”逐字节相同）。
#[must_use]
pub fn pan_automations(
    project: &YebanProjectV1,
    sample_rate: u32,
) -> BTreeMap<EntityId, TrackPanAutomation> {
    // `#297`：既有 `TrackPan` 泳道所在轨 ＋ 宏泳道映射到的轨。
    let mut wanted: BTreeSet<EntityId> = BTreeSet::new();
    for (track_id, track) in &project.tracks {
        if track
            .automation_lanes
            .keys()
            .any(|target| matches!(target, AutomationTarget::TrackPan { .. }))
        {
            wanted.insert(*track_id);
        }
    }
    for target in macro_mapped_targets(project) {
        if let AutomationTarget::TrackPan { track_id } = target {
            wanted.insert(track_id);
        }
    }
    let mut out = BTreeMap::new();
    for track_id in wanted {
        out.insert(
            track_id,
            TrackPanAutomation::new(project, track_id, sample_rate),
        );
    }
    out
}

/// R55 换域：`db_to_linear(自动化 dB) / db_to_linear(静态 dB)`。///
/// 分母为 0（静态推子下溢到 0）时比率不可表示 ⇒ 返回 `1.0`（退回静态推子），
/// 而不是造出 `inf`/`NaN` 再污染整段母带。
#[must_use]
fn automation_ratio(automation_db: f32, static_gain: f32) -> f32 {
    if !automation_db.is_finite() || static_gain <= 0.0 {
        return 1.0;
    }
    let ratio = db_to_linear(automation_db) / static_gain;
    if ratio.is_finite() { ratio } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr as _;

    use crate::inserts::{ConvolutionField, DeviceParamField, ReverbField, StripField};
    use yeban_model::{
        AutomationLane, AutomationPoint, AutomationWriteMode, CurveType, DeviceDefinition,
        DeviceKind, MacroMapping, MacroParameter, ParameterValue, RoutingEdge, RoutingKind,
        TrackKind,
    };

    fn point(id: u128, tick: u64, value: f32) -> AutomationPoint {
        let text = format!("01J8Z5Q0R7K3M9X2V4B6N8{id:04}");
        AutomationPoint {
            id: EntityId::from_str(&text).expect("合法 ULID"),
            tick,
            value,
            curve: CurveType::Linear,
        }
    }

    fn project_with_lane(values: &[(u64, f32)], read_enabled: bool) -> (YebanProjectV1, EntityId) {
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P001").expect("ULID");
        let mut project = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            volume_db: -6.0,
            ..TrackV3::default()
        };
        if !values.is_empty() {
            let target = AutomationTarget::TrackVolume { track_id };
            let mut points = std::collections::BTreeMap::new();
            for (index, (tick, value)) in values.iter().enumerate() {
                let point = point(index as u128, *tick, *value);
                points.insert(point.id, point);
            }
            track.automation_lanes.insert(
                target,
                AutomationLane {
                    target,
                    points,
                    read_enabled,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        }
        project.tracks.insert(track_id, track);
        (project, track_id)
    }

    /// 一条 `TrackVolume` 目标 ＋ 一条映射到它的 `Macro` 泳道（任务单 `#297`）。
    fn project_with_macro_lane(
        lane_value: f32,
        depth: f32,
        read_enabled: bool,
    ) -> (YebanProjectV1, EntityId) {
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P001").expect("ULID");
        let mut project = YebanProjectV1::default();
        let volume = AutomationTarget::TrackVolume { track_id };
        let macro_target = AutomationTarget::Macro {
            track_id,
            macro_index: 0,
        };
        let mut points = std::collections::BTreeMap::new();
        let only = point(0, 0, lane_value);
        points.insert(only.id, only);
        let mut track = TrackV3 {
            id: track_id,
            volume_db: -6.0,
            ..TrackV3::default()
        };
        track.macros = vec![MacroParameter {
            name: "Macro".to_owned(),
            value: 0.0,
            mappings: vec![MacroMapping {
                target: volume,
                depth,
            }],
        }];
        track.automation_lanes.insert(
            macro_target,
            AutomationLane {
                target: macro_target,
                points,
                read_enabled,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
        project.tracks.insert(track_id, track);
        (project, track_id)
    }

    /// ⭐ `#297` 判据 2/3：宏泳道按**目标单位**级联；`macro_value = 0` 时逐位恒等。
    #[test]
    fn a_macro_lane_cascades_in_target_units_and_zero_is_identity() {
        // depth = 6 dB、宏位置 1.0、静态推子 −6 dB ⇒ 有效值 0 dB
        // ⇒ 比率 = db_to_linear(0) / db_to_linear(-6)。
        let (project, track_id) = project_with_macro_lane(1.0, 6.0, true);
        let mut automation = TrackGainAutomation::new(&project, track_id, 48_000);
        assert!(!automation.is_identity(), "有非零宏级联 ⇒ 不得走恒等快路径");
        assert!(automation.is_evaluable());
        let expected = db_to_linear(0.0) / db_to_linear(-6.0);
        let mut buffer = vec![0.25_f32; 256];
        automation.apply_ratio_block(&mut buffer, 0, 256, 1);
        assert!(
            (buffer[0] - 0.25 * expected).abs() < 1e-6,
            "首样本 {} 应等于 0.25 × {expected}",
            buffer[0]
        );

        // 正对照：同一条宏泳道，宏位置恒 0 ⇒ 与"没有宏泳道"逐位相同。
        let (zero_project, zero_track) = project_with_macro_lane(0.0, 6.0, true);
        let zero = TrackGainAutomation::new(&zero_project, zero_track, 48_000);
        assert!(zero.is_identity(), "全 0 的宏点必须走恒等快路径");
        let mut untouched = vec![0.25_f32; 256];
        let mut zero = zero;
        zero.apply_ratio_block(&mut untouched, 0, 256, 1);
        assert_eq!(untouched, vec![0.25_f32; 256], "恒等时一个样本都不许动");

        // 关掉的宏泳道 = 没有宏值 ⇒ 同样恒等。
        let (disabled_project, disabled_track) = project_with_macro_lane(1.0, 6.0, false);
        assert!(TrackGainAutomation::new(&disabled_project, disabled_track, 48_000).is_identity());
    }

    /// ⭐ `#297` 判据 3/4：`Macro` 泳道的落点判定 —— 每条 mapping 都要落点，
    /// 空 mappings 与自指 mapping 一律报 `macro` 且**不得递归**（自指会栈溢出）。
    #[test]
    fn macro_lane_lands_only_when_every_mapping_lands() {
        let (project, track_id) = project_with_macro_lane(1.0, 6.0, true);
        let target = AutomationTarget::Macro {
            track_id,
            macro_index: 0,
        };
        let source_nodes = [track_id];
        let rendered_nodes = [track_id];
        assert_eq!(
            unsupported_lane_reason(&project, &target, &source_nodes, &[], &rendered_nodes),
            None,
            "TrackVolume 在计划里 ⇒ 宏泳道有落点"
        );

        let mut empty = project.clone();
        empty.tracks.get_mut(&track_id).expect("track").macros[0]
            .mappings
            .clear();
        assert_eq!(
            unsupported_lane_reason(&empty, &target, &source_nodes, &[], &rendered_nodes),
            Some(LaneResidual::Macro),
            "空 mappings 没有落点"
        );

        let mut self_reference = project.clone();
        self_reference
            .tracks
            .get_mut(&track_id)
            .expect("track")
            .macros[0]
            .mappings = vec![MacroMapping { target, depth: 1.0 }];
        // 自指：必须直接报 macro，而不是递归到栈溢出。
        assert_eq!(
            unsupported_lane_reason(
                &self_reference,
                &target,
                &source_nodes,
                &[],
                &rendered_nodes
            ),
            Some(LaneResidual::Macro)
        );
    }

    #[test]
    fn tick_at_frame_is_the_inverse_of_the_maestro_grid() {
        assert_eq!(tick_at_frame(0, 120.0, 48_000), 0);
        assert_eq!(tick_at_frame(48_000, 120.0, 48_000), 1_920);
        assert_eq!(tick_at_frame(96_000, 120.0, 48_000), 3_840);
        assert_eq!(tick_at_frame(128, 120.0, 48_000), 5);
    }

    #[test]
    fn an_empty_lane_is_the_identity() {
        let (project, track) = project_with_lane(&[], true);
        let automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 0);
    }

    #[test]
    fn a_disabled_lane_is_the_identity() {
        let (project, track) = project_with_lane(&[(0, -24.0), (1_920, 0.0)], false);
        let automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 2);
    }

    #[test]
    fn a_lane_that_is_flat_at_the_static_value_is_the_identity() {
        let (project, track) = project_with_lane(&[(0, -6.0), (1_920, -6.0)], true);
        let automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        assert!(automation.is_evaluable());
    }

    #[test]
    fn a_ramp_is_evaluated_and_moves_the_gain() {
        let (project, track) = project_with_lane(&[(0, -24.0), (1_920, 0.0)], true);
        let mut automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(!automation.is_identity());
        let mut out = vec![0.5f32; 2 * 96_000];
        automation.apply_ratio_block(&mut out, 0, 96_000, 2);
        let early = out[0];
        let late = out[(96_000 - 1) * 2];
        assert!(early < late, "渐强必须让后段更响: {early} vs {late}");
        assert!(automation.evaluated_blocks() > 0);
        assert_eq!(automation.last_db(), Some(0.0));
    }

    #[test]
    fn a_single_point_holds_its_value_everywhere() {
        let (project, track) = project_with_lane(&[(0, -12.0)], true);
        let mut automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(!automation.is_identity());
        let mut out = vec![1.0f32; 1_024];
        automation.apply_ratio_block(&mut out, 0, 1_024, 1);
        assert_eq!(out.len(), 1_024);
        let expected = db_to_linear(-12.0) / db_to_linear(-6.0);
        assert!(out.iter().all(|value| (*value - expected).abs() < 1e-6));
    }

    #[test]
    fn the_identity_fast_path_does_not_touch_negative_zero_or_nan_bits() {
        // 静态 -6 dB，泳道也处处 -6 dB ⇒ 恒等。样本里放 `-0.0`、带载荷的 NaN 与
        // 最小次正规数：乘 `1.0` 会改写 `-0.0` 的符号（`-0.0 * 1.0 == -0.0` 位型不变，
        // 但 `-0.0 + 0.0` 会变）；“整段跳过”对 NaN 载荷也恒等。
        let (project, track) = project_with_lane(&[(0, -6.0), (1_920, -6.0)], true);
        let mut automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        let mut out = vec![-0.0f32, f32::from_bits(0x7fc0_0001), f32::from_bits(1), 0.5];
        let before: Vec<u32> = out.iter().map(|value| value.to_bits()).collect();
        let len = out.len();
        automation.apply_ratio_block(&mut out, 0, len, 1);
        let after: Vec<u32> = out.iter().map(|value| value.to_bits()).collect();
        assert_eq!(before, after, "恒等快路径必须一个样本都不碰");
        assert_eq!(automation.evaluated_blocks(), 0);
    }

    #[test]
    fn a_muted_source_is_not_resurrected_by_a_zero_db_lane() {
        // 源侧静音时输出是 0；比率乘在 0 上仍是 0（分母用的是静态推子，不是 0）。
        let (project, track) = project_with_lane(&[(0, 0.0)], true);
        let mut automation = TrackGainAutomation::new(&project, track, 48_000);
        assert!(!automation.is_identity());
        let mut out = vec![0.0f32; 1_024];
        automation.apply_ratio_block(&mut out, 0, 1_024, 1);
        assert_eq!(out.len(), 1_024);
        assert!(out.iter().all(|value| value.to_bits() == 0));
    }

    /// `SendGain` 夹具：一条发送边（`edge_gain_db`）+ 一条轨上的泳道。
    fn project_with_send_lane(
        values: &[(u64, f32)],
        read_enabled: bool,
        edge_gain_db: Option<f32>,
    ) -> (YebanProjectV1, EntityId, EntityId) {
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P002").expect("ULID");
        let master = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8M001").expect("ULID");
        let edge_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8E002").expect("ULID");
        let mut project = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            ..TrackV3::default()
        };
        if !values.is_empty() {
            let target = AutomationTarget::SendGain { track_id, edge_id };
            let mut points = std::collections::BTreeMap::new();
            for (index, (tick, value)) in values.iter().enumerate() {
                let point = point(index as u128, *tick, *value);
                points.insert(point.id, point);
            }
            track.automation_lanes.insert(
                target,
                AutomationLane {
                    target,
                    points,
                    read_enabled,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        }
        project.tracks.insert(track_id, track);
        project.tracks.insert(
            master,
            TrackV3 {
                id: master,
                kind: TrackKind::Master,
                ..TrackV3::default()
            },
        );
        project.routing_graph.edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: track_id,
                destination_node: master,
                kind: RoutingKind::SendToAux,
                gain_db: edge_gain_db,
            },
        );
        (project, track_id, edge_id)
    }

    #[test]
    fn a_send_lane_with_no_points_is_the_identity() {
        let (project, track, edge) = project_with_send_lane(&[], true, None);
        let automation = SendGainAutomation::new(&project, track, edge, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 0);
        assert_eq!(automation.edge_id(), edge);
    }

    #[test]
    fn a_disabled_send_lane_is_the_identity() {
        let (project, track, edge) =
            project_with_send_lane(&[(0, -60.0), (1_920, 0.0)], false, None);
        let automation = SendGainAutomation::new(&project, track, edge, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 2);
    }

    #[test]
    fn a_send_lane_flat_at_the_static_edge_gain_is_the_identity() {
        // 静态边增益 -6 dB，泳道处处 -6 dB ⇒ 恒等（比较的是**线性增益**的位型）。
        let (project, track, edge) =
            project_with_send_lane(&[(0, -6.0), (1_920, -6.0)], true, Some(-6.0));
        let automation = SendGainAutomation::new(&project, track, edge, 48_000);
        assert!(automation.is_identity());
        assert!(automation.is_evaluable());
        assert!((automation.static_db() - -6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_send_ramp_moves_the_accumulated_gain_and_uses_the_model_entry_point() {
        let (project, track, edge) =
            project_with_send_lane(&[(0, -60.0), (1_920, 0.0)], true, None);
        let mut automation = SendGainAutomation::new(&project, track, edge, 48_000);
        assert!(!automation.is_identity());
        let src = vec![1.0f32; 2 * 96_000];
        let mut dst = vec![0.0f32; 2 * 96_000];
        automation.accumulate_block(&src, &mut dst, 0, 96_000, 2, 1.0);
        let early = dst[0];
        let late = dst[(96_000 - 1) * 2];
        assert!(early < late, "发送渐强必须让后段更响: {early} vs {late}");
        assert!(early > 0.0 && late <= 1.0, "增益落在静态边增益附近");
        assert!(automation.evaluated_blocks() > 0);
        assert_eq!(automation.last_db(), Some(0.0));
    }

    #[test]
    fn a_send_lane_whose_edge_is_missing_is_the_identity() {
        // 目标对账失败（边不存在）⇒ 不求值、不改字节（与 `#243` 的“关掉＝不存在”同口径）。
        let (mut project, track, edge) =
            project_with_send_lane(&[(0, -60.0), (1_920, 0.0)], true, None);
        project.routing_graph.edges.remove(&edge);
        let mut automation = SendGainAutomation::new(&project, track, edge, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());
        let src = vec![0.25f32; 512];
        let mut dst = vec![0.0f32; 512];
        automation.accumulate_block(&src, &mut dst, 0, 256, 2, 1.0);
        assert_eq!(dst, src, "恒等快路径必须只做静态累加");
    }

    #[test]
    fn send_gain_automations_finds_one_evaluator_per_edge() {
        let (project, _track, edge) =
            project_with_send_lane(&[(0, -60.0), (1_920, 0.0)], true, None);
        let automations = send_gain_automations(&project, 48_000);
        assert_eq!(automations.len(), 1, "只有带泳道的那条边被接上");
        assert!(!automations[&edge].is_identity());
    }

    /// `TrackPan` 夹具：一条轨（`track_pan`）+ 可选泳道。
    fn project_with_pan_lane(
        values: &[(u64, f32)],
        read_enabled: bool,
        track_pan: f32,
    ) -> (YebanProjectV1, EntityId) {
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P003").expect("ULID");
        let mut project = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            pan: track_pan,
            ..TrackV3::default()
        };
        if !values.is_empty() {
            let target = AutomationTarget::TrackPan { track_id };
            let mut points = std::collections::BTreeMap::new();
            for (index, (tick, value)) in values.iter().enumerate() {
                let point = point(index as u128, *tick, *value);
                points.insert(point.id, point);
            }
            track.automation_lanes.insert(
                target,
                AutomationLane {
                    target,
                    points,
                    read_enabled,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        }
        project.tracks.insert(track_id, track);
        (project, track_id)
    }

    /// 测试用的曲线：与 `yeban_engine::mixer::pan_gains` 同式（`θ = (p+1)·π/4`）。
    fn curve(pan: f32) -> (f32, f32) {
        let angle = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
        (libm::cosf(angle), libm::sinf(angle))
    }

    #[test]
    fn an_empty_or_disabled_pan_lane_is_the_identity() {
        let (project, track) = project_with_pan_lane(&[], true, 0.0);
        let automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        assert!(!automation.is_evaluable());

        let (project, track) = project_with_pan_lane(&[(0, -1.0), (1_920, 1.0)], false, 0.0);
        let automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity(), "readEnabled:false 等于没有泳道");
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 2);
    }

    #[test]
    fn a_pan_lane_flat_at_the_static_pan_is_the_identity() {
        let (project, track) = project_with_pan_lane(&[(0, -0.5), (1_920, -0.5)], true, -0.5);
        let automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        assert!(automation.is_evaluable());
        assert_eq!(automation.static_pan(), -0.5);
    }

    #[test]
    fn a_pan_ramp_moves_left_to_right() {
        let (project, track) = project_with_pan_lane(&[(0, -1.0), (1_920, 1.0)], true, 0.0);
        let mut automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(!automation.is_identity());
        let frames = 96_000usize;
        let mut out = vec![0.5f32; 2 * frames];
        automation.apply_pan_block(&mut out, 0, frames, 2, curve);
        let first = (out[0], out[1]);
        let last = (out[(frames - 1) * 2], out[(frames - 1) * 2 + 1]);
        println!("### pan ramp: first={first:?} last={last:?}");
        assert!(first.0 > first.1, "起点在全左：左必须大于右: {first:?}");
        assert!(last.1 > last.0, "终点在全右：右必须大于左: {last:?}");
        assert!(automation.evaluated_blocks() > 0);
        assert_eq!(automation.last_pan(), Some(1.0));
    }

    #[test]
    fn a_constant_pan_lane_matches_the_static_pan_curve() {
        let (project, track) = project_with_pan_lane(&[(0, 0.5)], true, 0.0);
        let mut automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(!automation.is_identity());
        let mut out = vec![1.0f32; 2 * 1_024];
        automation.apply_pan_block(&mut out, 0, 1_024, 2, curve);
        let (left, right) = centered_pan_gains(0.5, curve);
        assert_eq!(out[0].to_bits(), left.to_bits(), "左 = 静态轨同一条曲线");
        assert_eq!(out[1].to_bits(), right.to_bits(), "右 = 静态轨同一条曲线");
        // 中心是**不衰减**（与静态 `track_pan_gains` 同一条规则），⛔ 不是 √2/2。
        assert_eq!(centered_pan_gains(0.0, curve), (1.0, 1.0));
        assert_eq!(centered_pan_gains(f32::NAN, curve), (1.0, 1.0));
    }

    #[test]
    fn the_pan_identity_fast_path_does_not_touch_bits() {
        let (project, track) = project_with_pan_lane(&[(0, -0.5), (1_920, -0.5)], true, -0.5);
        let mut automation = TrackPanAutomation::new(&project, track, 48_000);
        assert!(automation.is_identity());
        let mut out = vec![-0.0f32, f32::from_bits(0x7fc0_0001), f32::from_bits(1), 0.5];
        let before: Vec<u32> = out.iter().map(|value| value.to_bits()).collect();
        let len = out.len();
        automation.apply_pan_block(&mut out, 0, len, 2, curve);
        let after: Vec<u32> = out.iter().map(|value| value.to_bits()).collect();
        assert_eq!(before, after, "恒等快路径必须一个样本都不碰");
        assert_eq!(automation.evaluated_blocks(), 0);
    }

    #[test]
    fn pan_automations_finds_one_evaluator_per_track() {
        let (project, track) = project_with_pan_lane(&[(0, -1.0), (1_920, 1.0)], true, 0.0);
        let automations = pan_automations(&project, 48_000);
        assert_eq!(automations.len(), 1, "只有带泳道的那条轨被接上");
        assert!(!automations[&track].is_identity());
    }

    // -----------------------------------------------------------------------
    // ⭐ `#266`（一个名字）／`#267`（通道条整组）：`DeviceParam` 自动化
    // -----------------------------------------------------------------------

    /// 一条轨 + 一台设备（给定的参数表）+ 可选的 `DeviceParam { slot: 0, param: 0 }` 泳道。
    fn project_with_device_lane(
        params: &[(&str, f32)],
        values: &[(u64, f32)],
        read_enabled: bool,
        bypassed: bool,
    ) -> (YebanProjectV1, EntityId) {
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P004").expect("ULID");
        let device_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8D001").expect("ULID");
        let mut project = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            devices: vec![DeviceDefinition {
                id: device_id,
                name: "strip".to_owned(),
                kind: DeviceKind::InternalEffect,
                bypassed,
                params: params
                    .iter()
                    .map(|(name, value)| ParameterValue {
                        name: (*name).to_owned(),
                        value: *value,
                        unit: None,
                    })
                    .collect(),
                latency_samples: 0,
            }],
            ..TrackV3::default()
        };
        if !values.is_empty() {
            let target = AutomationTarget::DeviceParam {
                track_id,
                slot_index: 0,
                param_index: 0,
            };
            let mut points = std::collections::BTreeMap::new();
            for (index, (tick, value)) in values.iter().enumerate() {
                let point = point(index as u128, *tick, *value);
                points.insert(point.id, point);
            }
            track.automation_lanes.insert(
                target,
                AutomationLane {
                    target,
                    points,
                    read_enabled,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        }
        project.tracks.insert(track_id, track);
        (project, track_id)
    }

    #[test]
    fn a_device_param_ramp_is_evaluated_once_per_block_and_touches_no_sample() {
        // 设备只写 `output_gain_db: 0.0` ⇒ 静态投影是恒等（`from_devices` 的
        // `params != DEFAULT` 兜底不认它）——这正是"强制实例化"要覆盖的缝。
        let (project, track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (1_920, -12.0)],
            true,
            false,
        );
        let mut automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(automation.is_applicable(), "全恒等设备也是来源");
        assert!(automation.is_evaluable());
        assert!(!automation.is_identity());
        assert_eq!(automation.points(), 2);
        assert_eq!(automation.static_value().to_bits(), 0.0f32.to_bits());
        assert_eq!(automation.param_name(), "output_gain_db");
        // 块 0：曲线起点（⛔ 不"淡入"）。
        assert_eq!(
            automation.value_at_block(0).expect("块 0").to_bits(),
            0.0f32.to_bits()
        );
        // 24 000 帧 = 0.5 s = 960 tick ⇒ 曲线中点 −6 dB。
        let mid = automation.value_at_block(24_000).expect("中点");
        assert!((mid - -6.0).abs() < 1e-5, "中点必须是 −6 dB，实测 {mid}");
        // 96 000 帧 = 1 s = 1920 tick ⇒ 末点（保持）。
        assert_eq!(
            automation.value_at_block(96_000).expect("末端").to_bits(),
            (-12.0f32).to_bits()
        );
        assert_eq!(automation.evaluated_blocks(), 3);
        assert_eq!(automation.last_value(), Some(-12.0));
        automation.reset_for_execution();
        assert_eq!(automation.evaluated_blocks(), 0);
        assert_eq!(automation.last_value(), None);
    }

    #[test]
    fn a_disabled_or_flat_device_param_lane_is_the_identity() {
        let (project, track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (1_920, -12.0)],
            false,
            false,
        );
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(automation.is_identity(), "readEnabled:false 等于没有泳道");
        assert!(!automation.is_evaluable());
        assert_eq!(automation.points(), 2);

        let (project, track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (960, 0.0), (1_920, 0.0)],
            true,
            false,
        );
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(automation.is_identity(), "处处等于静态值 ⇒ 恒等");
        assert!(automation.is_evaluable());
        assert!(automation.is_applicable());

        // `output_gain_db: -60` 投影后钳到 -60；`-100` 钳到同一个值 ⇒ 仍恒等。
        let (project, track) =
            project_with_device_lane(&[("output_gain_db", -60.0)], &[(0, -100.0)], true, false);
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(automation.is_identity(), "恒等比的是**钳制后**的值");
    }

    #[test]
    fn only_the_strip_source_device_can_be_automated() {
        // 槽位 0 是通道条来源（-6 dB 让 `strip_of` 认它）；槽位 1 也是同名列的
        // 另一台设备 ⇒ 它的参数不进 DSP ⇒ 泳道不落点。
        let track_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P005").expect("ULID");
        let mut project = YebanProjectV1::default();
        let mut track = TrackV3 {
            id: track_id,
            devices: vec![
                DeviceDefinition {
                    id: EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8D001").expect("ULID"),
                    name: "first".to_owned(),
                    kind: DeviceKind::InternalEffect,
                    bypassed: false,
                    params: vec![ParameterValue {
                        name: "output_gain_db".to_owned(),
                        value: -6.0,
                        unit: None,
                    }],
                    latency_samples: 0,
                },
                DeviceDefinition {
                    id: EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8D002").expect("ULID"),
                    name: "second".to_owned(),
                    kind: DeviceKind::InternalEffect,
                    bypassed: false,
                    params: vec![ParameterValue {
                        name: "output_gain_db".to_owned(),
                        value: -3.0,
                        unit: None,
                    }],
                    latency_samples: 0,
                },
            ],
            ..TrackV3::default()
        };
        let target = AutomationTarget::DeviceParam {
            track_id,
            slot_index: 1,
            param_index: 0,
        };
        let lane = AutomationLane {
            target,
            points: std::collections::BTreeMap::new(),
            read_enabled: true,
            write_mode: AutomationWriteMode::Off,
            domain: None,
        };
        track.automation_lanes.insert(target, lane);
        project.tracks.insert(track_id, track);
        assert!(!device_param_lane_is_applicable(&project, &target));
        let automation = DeviceParamAutomation::new(&project, track_id, 1, 0, 48_000);
        assert!(automation.is_identity(), "非来源设备 ⇒ 恒等（不落点）");

        let source = AutomationTarget::DeviceParam {
            track_id,
            slot_index: 0,
            param_index: 0,
        };
        assert!(device_param_lane_is_applicable(&project, &source));
        assert!(!device_param_lane_is_applicable(
            &project,
            &AutomationTarget::TrackVolume { track_id }
        ));
    }

    #[test]
    fn a_bypassed_or_unapplied_device_param_lane_is_never_applicable() {
        // 旁通设备：`from_devices` 当它不存在 ⇒ 泳道也不落点。
        let (project, track) = project_with_device_lane(
            &[("output_gain_db", -6.0)],
            &[(0, 0.0), (1_920, -12.0)],
            true,
            true,
        );
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(!automation.is_applicable());
        assert!(automation.is_identity());

        // 白名单外的名字（`mix`）：解析成功、名字仍报，但不落点。
        let (project, track) =
            project_with_device_lane(&[("mix", 0.5)], &[(0, 1.0), (1_920, 0.0)], true, false);
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(!automation.is_applicable());
        assert!(automation.is_identity());
        assert_eq!(automation.param_name(), "");
    }

    #[test]
    fn the_scanner_forces_a_strip_only_for_non_identity_device_param_lanes() {
        let (ramp, track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (1_920, -12.0)],
            true,
            false,
        );
        let automations = device_param_automations(&ramp, 48_000);
        assert_eq!(automations.len(), 1);
        assert_eq!(automations[&track].len(), 1);
        assert!(!automations[&track][0].is_identity());
        assert_eq!(
            forced_strip_tracks(&automations),
            std::collections::BTreeSet::from([track]),
            "有非恒等泳道 ⇒ 必须强制实例化通道条"
        );

        let (flat, track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (1_920, 0.0)],
            true,
            false,
        );
        let automations = device_param_automations(&flat, 48_000);
        assert!(automations[&track][0].is_identity());
        assert!(
            forced_strip_tracks(&automations).is_empty(),
            "恒等泳道不强制实例化（否则产物会变）"
        );

        let (disabled, _track) = project_with_device_lane(
            &[("output_gain_db", 0.0)],
            &[(0, 0.0), (1_920, -12.0)],
            false,
            false,
        );
        let automations = device_param_automations(&disabled, 48_000);
        assert!(forced_strip_tracks(&automations).is_empty());
    }

    /// ⭐ `#293` 判据 3：强制实例化**按族收窄** —— 混响/卷积泳道不许白实例化通道条，
    /// 反之亦然；恒等泳道三族都不置位。
    #[test]
    fn the_scanner_forces_each_family_separately() {
        let (reverb_project, reverb_track) = project_with_device_lane(
            &[("reverb_wet", 0.0)],
            &[(0, 0.0), (1_920, 1.0)],
            true,
            false,
        );
        let automations = device_param_automations(&reverb_project, 48_000);
        let forced = forced_insert_instances(&automations);
        assert_eq!(
            forced.reverb,
            BTreeSet::from([reverb_track]),
            "非恒等混响泳道必须强制实例化 Freeverb"
        );
        assert!(forced.strip.is_empty(), "混响泳道不许白实例化通道条");
        assert!(forced.convolution.is_empty());
        assert!(
            forced_strip_tracks(&automations).is_empty(),
            "`forced_strip_tracks` 是同一扫描器的 strip 族投影"
        );

        let (conv_project, conv_track) =
            project_with_device_lane(&[("conv_wet", 0.0)], &[(0, 0.0), (1_920, 1.0)], true, false);
        let automations = device_param_automations(&conv_project, 48_000);
        let forced = forced_insert_instances(&automations);
        assert_eq!(forced.convolution, BTreeSet::from([conv_track]));
        assert!(forced.strip.is_empty() && forced.reverb.is_empty());

        let (flat_project, _flat_track) = project_with_device_lane(
            &[("reverb_wet", 0.25)],
            &[(0, 0.25), (1_920, 0.25)],
            true,
            false,
        );
        let automations = device_param_automations(&flat_project, 48_000);
        assert!(
            forced_insert_instances(&automations).is_empty(),
            "恒等泳道三族都不许强制实例化（否则产物会变）"
        );
    }

    // -----------------------------------------------------------------------
    // ⭐ `#267`/`#293`：通道条整组（27 拼写 = 14 字段）＋ 混响/卷积标量（14 拼写 = 8 字段）
    // -----------------------------------------------------------------------

    /// 每个字段的：规范名、静态值、渐变的终点值。
    ///
    /// 静态值刻意**全非默认**（除了两个开关的 `0` 与两个增益的 `0` —— 那三组正是
    /// `#266` 的兜底缝，单独一条判据覆盖）。
    const FIELD_CASES: &[(StripField, &str, f32, f32)] = &[
        (StripField::InputGainDb, "input_gain_db", 0.0, -12.0),
        (StripField::OutputGainDb, "output_gain_db", 0.0, -12.0),
        (StripField::EqEnabled, "eq_enabled", 0.0, 1.0),
        (StripField::FilterEnabled, "filter_enabled", 0.0, 1.0),
        (StripField::EqLowGain, "eq_low_gain", 3.0, -6.0),
        (StripField::EqLowFreq, "eq_low_freq", 800.0, 4_000.0),
        (StripField::EqMidGain, "eq_mid_gain", -3.0, 6.0),
        (StripField::EqMidFreq, "eq_mid_freq", 2_000.0, 500.0),
        (StripField::EqMidQ, "eq_mid_q", 1.5, 0.4),
        (StripField::EqHighGain, "eq_high_gain", -3.0, 6.0),
        (StripField::EqHighFreq, "eq_high_freq", 6_000.0, 1_500.0),
        (StripField::CutoffHz, "cutoff_hz", 5_000.0, 800.0),
        (StripField::Resonance, "resonance", 0.2, 0.9),
        (StripField::Drive, "drive", 0.2, 0.9),
    ];

    /// 非通道条字段（`#293`）的：落点、规范名、静态值、渐变终点值。
    ///
    /// 静态值都**不是器件默认值的重复**（`reverb_wet: 0` 正是"静态链不建器件"的缝，
    /// 单列一条运行态判据）；`conv_ir_gain_db` 用 `0 dB → −12 dB`。
    const INSERT_FIELD_CASES: &[(DeviceParamField, &str, f32, f32)] = &[
        (
            DeviceParamField::Reverb(ReverbField::Size),
            "reverb_size",
            0.2,
            0.9,
        ),
        (
            DeviceParamField::Reverb(ReverbField::Damp),
            "reverb_damp",
            0.1,
            0.9,
        ),
        (
            DeviceParamField::Reverb(ReverbField::Wet),
            "reverb_wet",
            0.0,
            1.0,
        ),
        (
            DeviceParamField::Reverb(ReverbField::Predelay),
            "reverb_predelay_s",
            0.0,
            0.09,
        ),
        (
            DeviceParamField::Convolution(ConvolutionField::PreDelay),
            "conv_predelay_s",
            0.0,
            0.09,
        ),
        (
            DeviceParamField::Convolution(ConvolutionField::Dry),
            "conv_dry",
            1.0,
            0.0,
        ),
        (
            DeviceParamField::Convolution(ConvolutionField::Wet),
            "conv_wet",
            0.0,
            1.0,
        ),
        (
            DeviceParamField::Convolution(ConvolutionField::IrGainDb),
            "conv_ir_gain_db",
            0.0,
            -12.0,
        ),
    ];

    /// 一个**名单拼写**的用例：落点、规范名、静态值、终点值。
    fn case_for(name: &str) -> (DeviceParamField, &'static str, f32, f32) {
        let field = DeviceParamField::from_name(name)
            .unwrap_or_else(|| panic!("{name} 在名单里但解析不出落点"));
        match field {
            DeviceParamField::Strip(strip) => FIELD_CASES
                .iter()
                .find(|case| case.0 == strip)
                .map(|&(_, canonical, from, to)| (field, canonical, from, to))
                .expect("每个通道条字段都有用例"),
            _ => INSERT_FIELD_CASES
                .iter()
                .find(|case| case.0 == field)
                .map(|&(_, canonical, from, to)| (field, canonical, from, to))
                .expect("每个插入字段都有用例"),
        }
    }

    /// ⭐ 名单 ↔ 字段表**双向**一致：`APPLIED_DEVICE_PARAM_NAMES` 就是
    /// `StripField::from_name` 认下的那一组拼写（⛔ 只改一边会让两条判据红）。
    #[test]
    fn the_applied_names_and_the_field_table_agree() {
        // 1. 名单里的每个拼写都能解析成一个落点，且**规范名**也在名单里。
        for &name in APPLIED_DEVICE_PARAM_NAMES {
            let field = DeviceParamField::from_name(name)
                .unwrap_or_else(|| panic!("{name} 在名单里但解析不出落点"));
            assert!(
                is_applied_device_param_name(&name.to_ascii_uppercase()),
                "大小写不敏感：{name}"
            );
            assert!(
                APPLIED_DEVICE_PARAM_NAMES
                    .iter()
                    .any(|applied| applied.eq_ignore_ascii_case(field.canonical_name())),
                "字段 {} 的规范名必须在名单里",
                field.canonical_name()
            );
        }
        // 2. 三族各自的每个字段都有一条规范拼写在名单里，且拼写集合包含它自己。
        for field in StripField::ALL {
            let canonical = field.canonical_name();
            assert!(
                APPLIED_DEVICE_PARAM_NAMES.contains(&canonical),
                "{canonical}"
            );
            let spellings: Vec<&str> = APPLIED_DEVICE_PARAM_NAMES
                .iter()
                .copied()
                .filter(|name| StripField::from_name(name) == Some(*field))
                .collect();
            assert!(spellings.contains(&canonical), "{canonical}: {spellings:?}");
        }
        for field in ReverbField::ALL {
            let canonical = field.canonical_name();
            assert!(
                APPLIED_DEVICE_PARAM_NAMES.contains(&canonical),
                "{canonical}"
            );
            let spellings: Vec<&str> = APPLIED_DEVICE_PARAM_NAMES
                .iter()
                .copied()
                .filter(|name| ReverbField::from_name(name) == Some(*field))
                .collect();
            assert!(spellings.contains(&canonical), "{canonical}: {spellings:?}");
        }
        for field in ConvolutionField::ALL {
            let canonical = field.canonical_name();
            assert!(
                APPLIED_DEVICE_PARAM_NAMES.contains(&canonical),
                "{canonical}"
            );
            let spellings: Vec<&str> = APPLIED_DEVICE_PARAM_NAMES
                .iter()
                .copied()
                .filter(|name| ConvolutionField::from_name(name) == Some(*field))
                .collect();
            assert!(spellings.contains(&canonical), "{canonical}: {spellings:?}");
        }
        // 3. 名单与三张**唯一名字表**的并集逐字相等（⛔ 少一个、多一个、重复都红）。
        assert_eq!(StripField::ALL.len(), 14, "通道条 14 个字段");
        assert_eq!(ReverbField::ALL.len(), 4, "Freeverb 4 个字段");
        assert_eq!(ConvolutionField::ALL.len(), 4, "卷积标量 4 个字段");
        let expected_len = StripField::SPELLINGS.len()
            + ReverbField::SPELLINGS.len()
            + ConvolutionField::SPELLINGS.len();
        assert_eq!(expected_len, 41, "27 + 6 + 8 = 41 个拼写");
        assert_eq!(
            APPLIED_DEVICE_PARAM_NAMES.len(),
            expected_len,
            "名单与三张 SPELLINGS 的并集条数必须相等"
        );
        let mut sorted: Vec<&str> = APPLIED_DEVICE_PARAM_NAMES.to_vec();
        sorted.sort_unstable();
        let mut expected: Vec<&str> = StripField::SPELLINGS
            .iter()
            .map(|(name, _)| *name)
            .chain(ReverbField::SPELLINGS.iter().map(|(name, _)| *name))
            .chain(ConvolutionField::SPELLINGS.iter().map(|(name, _)| *name))
            .collect();
        expected.sort_unstable();
        assert_eq!(sorted, expected, "名单必须与三张 SPELLINGS 完全一致");
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            APPLIED_DEVICE_PARAM_NAMES.len(),
            "名单里不许有重复拼写"
        );
        // 4. 名单外的名字一律不认。
        for name in [
            "threshold_db",
            "ratio",
            "knee_db",
            "detector_s",
            "attack_s",
            "release_s",
            "makeup_db",
            "compressor_enabled",
            "reverb_mix",
            "reverb_width",
            "conv_ir_decay_s",
            "conv_ir_decay",
            "conv_ir_seed",
            "mix",
            "filter_cutoff_hz",
        ] {
            assert!(!is_applied_device_param_name(name), "{name} 不该有落点");
        }
    }

    /// ⭐ `#267`/`#293` 判据 1/2：**22 个字段各一条**（41 个拼写逐个跑）—— 泳道从静态值
    /// 渐变到终点值 ⇒ 求值器把**这个字段**接上（`field()` / `param_name()` / 静态值），
    /// 曲线在块边界上单调，且恒等判定为假。别名与规范名解析到**同一个**落点。
    #[test]
    fn every_applied_name_is_applied_and_ramps_monotonically() {
        for &name in APPLIED_DEVICE_PARAM_NAMES {
            let (field, _canonical, static_value, target) = case_for(name);
            let (project, track) = project_with_device_lane(
                &[(name, static_value)],
                &[(0, static_value), (1_920, target)],
                true,
                false,
            );
            let mut automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(automation.is_applicable(), "{name} 必须有落点");
            assert_eq!(automation.field(), Some(field), "{name} 的字段");
            assert_eq!(automation.param_name(), name, "{name} 的原文");
            assert_eq!(
                automation.static_value().to_bits(),
                field.sanitise(static_value).to_bits(),
                "{name} 的静态值必须与器件同一次钳制"
            );
            assert!(automation.is_evaluable(), "{name} 有可求值点");
            assert!(!automation.is_identity(), "{name} 渐变必须非恒等");
            // 块边界上的读数：起点 == 静态值、终点 == 终点值、中点严格在两者之间。
            let start = automation.value_at_block(0).expect("块 0");
            assert_eq!(start.to_bits(), field.sanitise(static_value).to_bits());
            let mid = automation.value_at_block(24_000).expect("中点");
            let end = automation.value_at_block(96_000).expect("末端");
            assert_eq!(end.to_bits(), target.to_bits(), "{name} 末端");
            let low = static_value.min(target);
            let high = static_value.max(target);
            assert!(
                (low..=high).contains(&mid),
                "{name} 中点必须在两端之间: {mid}"
            );
            if (target - static_value).abs() > 1e-3 {
                assert!(
                    mid != start && mid != end,
                    "{name} 的曲线中点必须真的在移动: {mid}"
                );
            }
            assert_eq!(automation.evaluated_blocks(), 3);
            assert_eq!(automation.last_value(), Some(end));
        }
    }

    /// ⭐ `#267` 判据 3/4：**每个字段**的恒等路径 —— 泳道处处等于静态值、或
    /// `readEnabled:false` ⇒ [`DeviceParamAutomation::is_identity`] 为真
    /// （⇒ 不强制实例化器件、一个参数都不改）。
    #[test]
    fn every_applied_name_is_identity_when_flat_or_disabled() {
        for &name in APPLIED_DEVICE_PARAM_NAMES {
            let (field, _canonical, static_value, target) = case_for(name);
            let flat = [static_value, static_value, static_value];
            let (project, track) = project_with_device_lane(
                &[(name, static_value)],
                &[(0, flat[0]), (960, flat[1]), (1_920, flat[2])],
                true,
                false,
            );
            let mut automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(automation.is_applicable(), "{name} 有落点");
            assert!(automation.is_identity(), "{name} 处处等于静态值 ⇒ 恒等");
            assert_eq!(automation.field(), Some(field));
            assert!(
                automation.value_at_block(0).is_none(),
                "{name} 恒等不改参数"
            );

            let (project, track) = project_with_device_lane(
                &[(name, static_value)],
                &[(0, static_value), (1_920, target)],
                false,
                false,
            );
            let mut automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(automation.is_identity(), "{name} 的 readEnabled:false");
            assert!(!automation.is_evaluable());
            assert!(automation.value_at_block(0).is_none());
        }
    }

    /// ⭐ `#267`/`#293`：**钳制后**才比恒等（`-100 dB` 与 `-60 dB` 交给器件是同一个值）；
    /// 并且名单外的名字（压缩 / 卷积 IR 形状 / 未知旋钮）即使指向合法设备也**不落点**
    /// （`field()` 是 `None`）。
    #[test]
    fn the_identity_compares_the_clamped_value_and_rejects_other_names() {
        let (project, track) =
            project_with_device_lane(&[("output_gain_db", -60.0)], &[(0, -100.0)], true, false);
        let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
        assert!(automation.is_identity(), "钳制后相等 ⇒ 恒等");
        assert_eq!(
            automation.field(),
            Some(DeviceParamField::Strip(StripField::OutputGainDb))
        );

        for name in ["threshold_db", "conv_ir_decay_s", "conv_ir_seed", "mix"] {
            let (project, track) =
                project_with_device_lane(&[(name, 1.0)], &[(0, 0.0), (960, 2.0)], true, false);
            let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(!automation.is_applicable(), "{name} 不该有落点");
            assert!(automation.is_identity());
            assert_eq!(automation.field(), None);
            assert_eq!(automation.param_name(), "");
        }
    }

    /// ⭐ `#266` 的兜底缝在 `#267` 之后**仍然成立**：只写默认值的**增益**设备投影成
    /// `None`（`params != DEFAULT` 不认它），但它的泳道仍然是来源 —— 静态基值就是
    /// [`forced_identity_strip`]。
    ///
    /// ⚠ 缝只对两个增益字段存在：其它字段只要出现就把 `seen` 位置起 ⇒ 投影是 `Some`。
    #[test]
    fn the_identity_seam_still_holds_for_the_gain_fields() {
        for name in [
            "output_gain_db",
            "input_gain_db",
            "output_gain",
            "input_gain",
            "trim_db",
        ] {
            let (project, track) =
                project_with_device_lane(&[(name, 0.0)], &[(0, 0.0), (1_920, 6.0)], true, false);
            let track_ref = &project.tracks[&track];
            assert!(
                strip_source_params(track_ref).is_none(),
                "{name}: 全默认设备必须落进兜底缝（投影是 None）"
            );
            assert_eq!(
                strip_source_slot(track_ref),
                None,
                "{name}: 缝里没有投影来源"
            );
            let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(automation.is_applicable(), "{name}: 缝里仍然有落点");
            assert!(!automation.is_identity(), "{name}: 渐变非恒等 ⇒ 强制实例化");
            assert_eq!(
                automation.static_value().to_bits(),
                0.0f32.to_bits(),
                "{name}: 静态基值是恒等参数"
            );
        }
        // 对照：非增益字段写"默认值"也**不**落进缝里（`seen` 位把投影点起来）。
        for (name, value) in [
            ("cutoff_hz", 20_000.0),
            ("resonance", 0.0),
            ("drive", 0.0),
            ("eq_low_gain", 0.0),
            ("eq_enabled", 1.0),
            ("filter_enabled", 1.0),
        ] {
            let (project, track) =
                project_with_device_lane(&[(name, value)], &[(0, value)], true, false);
            assert!(
                strip_source_params(&project.tracks[&track]).is_some(),
                "{name}: 非增益字段必须被投影认下（不进兜底缝）"
            );
            let automation = DeviceParamAutomation::new(&project, track, 0, 0, 48_000);
            assert!(automation.is_applicable(), "{name}: 有落点");
            assert!(
                automation.is_identity(),
                "{name}: 泳道 == 静态值 ⇒ 恒等（不白实例化器件）"
            );
        }
    }
}
