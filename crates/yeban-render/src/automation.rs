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
//! （音量/发送比的是线性增益的位型，声相比的是位置的位型）⇒ `is_identity` 为真，
//! 整段跳过。
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
//! - 只接 [`AutomationTarget::TrackVolume`]（含主总线）、[`AutomationTarget::SendGain`]
//!   与 [`AutomationTarget::TrackPan`]；`DeviceParam` / `Macro` 仍由调用方如实登记为
//!   不支持；
//! - 主总线的第二个站点在调用方（主总线没有源，它的 `volume_db` 在源侧无法表达）；
//! - 发送量的落点在 [`crate::render::RenderPlan`] 的**入边归约**里（见
//!   [`SendGainAutomation`]）：它乘的是该边的**静态增益**，与源节点的 `TrackVolume`
//!   是两个不同的站点，互不叠加；
//! - 声相（[`TrackPanAutomation`]）的落点在调用方的**逐轨源输出之后**：调用方必须在
//!   非恒等时把源侧静态声相关掉（`pan = 0` ⇒ `(1, 1)`），否则会乘两次 —— 本模块只
//!   提供那个乘子，⛔ 不替调用方改源；
//! - 没有把“自动化值”写回快照或模型：这是**离线渲染的一次性读数**。

use std::collections::BTreeMap;

use yeban_dsp::smoothing::ParamSmoother;
use yeban_model::{AutomationTarget, EntityId, PPQ, TrackV3, YebanProjectV1};

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
        let (evaluable, points, identity) = match lane {
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
        if let Ok(Some(value)) = self.evaluator.automation_value_at(&self.target, 0) {
            let ratio = automation_ratio(value, self.static_gain);
            self.smoother.snap_to(ratio);
        }
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
        // 唯一入口：目标对账 + `read_enabled` 闸 + 曲线插值都在模型层。
        let value = self.evaluator.automation_value_at(&self.target, tick);
        let target = match value {
            Ok(Some(value)) if value.is_finite() => {
                self.last_db = Some(value);
                automation_ratio(value, self.static_gain)
            }
            // `Ok(None)`：泳道被关掉 / 空泳道；`Err(_)`：目标在文档里不存在。
            // 两者都表示“此刻没有自动化值” ⇒ 退回静态推子（比率 1.0）。
            _ => 1.0,
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
        let (evaluable, points, identity) = match lane {
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
        let start = match self.evaluator.automation_value_at(&self.target, 0) {
            Ok(Some(value)) if value.is_finite() => value,
            _ => self.static_pan,
        };
        self.smoother
            .snap_to(if start.is_finite() { start } else { 0.0 });
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
        // 唯一入口：目标对账 + `read_enabled` 闸 + 曲线插值都在模型层。
        let pan = match self.evaluator.automation_value_at(&self.target, tick) {
            Ok(Some(value)) if value.is_finite() => {
                self.last_pan = Some(value);
                value
            }
            // `Ok(None)` / `Err(_)`：此刻没有自动化值 ⇒ 退回静态声相（源侧已被关掉，
            // 所以这里必须是静态位置，而不是居中）。
            _ => self.static_pan,
        };
        self.smoother.set_target(pan);
    }
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
    let mut out = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for target in track.automation_lanes.keys() {
            if let AutomationTarget::SendGain { edge_id, .. } = target {
                out.insert(
                    *edge_id,
                    SendGainAutomation::new(project, *track_id, *edge_id, sample_rate),
                );
            }
        }
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
    let mut out = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        if track
            .automation_lanes
            .keys()
            .any(|target| matches!(target, AutomationTarget::TrackPan { .. }))
        {
            out.insert(
                *track_id,
                TrackPanAutomation::new(project, *track_id, sample_rate),
            );
        }
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

    use yeban_model::{
        AutomationLane, AutomationPoint, AutomationWriteMode, CurveType, RoutingEdge, RoutingKind,
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
}
