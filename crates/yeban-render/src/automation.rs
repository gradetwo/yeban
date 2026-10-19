//! 离线导出里的**音量自动化**：目标求值 → 线性增益 → 5 ms 平滑 [ARCH-DSP-001]。
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
//!
//! ## 恒等快路径（一个样本都不碰）
//!
//! 没有泳道、`read_enabled == false`、泳道没有点，或者**每个点的线性增益都等于静态
//! 推子的线性增益** ⇒ [`TrackGainAutomation::is_identity`] 为真，整段跳过。
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
//! - 只接 [`AutomationTarget::TrackVolume`]；`TrackPan` / `SendGain` / `DeviceParam` /
//!   `Macro` 仍由调用方如实登记为不支持；
//! - 主总线的第二个站点在调用方（主总线没有源，它的 `volume_db` 在源侧无法表达）；
//! - 没有把“自动化值”写回快照或模型：这是**离线渲染的一次性读数**。

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
        let target = AutomationTarget::TrackVolume { track_id };
        let static_db = project
            .tracks
            .get(&track_id)
            .map_or(0.0, |track| track.volume_db);
        let static_gain = db_to_linear(static_db);
        let lane = project.automation_lane(&target);
        let (evaluable, points, identity) = match lane {
            None => (false, 0, true),
            Some(lane) if !lane.read_enabled => (false, lane.points.len(), true),
            Some(lane) if lane.points.is_empty() => (false, 0, true),
            Some(lane) => {
                // “曲线值处处等于静态值”的充要条件：每个采样点上的**线性增益**都与
                // 静态推子逐位相同。分段插值在两端相等时整段恒定，边界是保持；
                // 因此只要所有点相等，整条曲线就处处相等。
                let identity = lane
                    .points
                    .values()
                    .all(|point| db_to_linear(point.value).to_bits() == static_gain.to_bits());
                (true, lane.points.len(), identity)
            }
        };
        // 最小求值工程：只放本轨，泳道键/值原样克隆（`automation_value_at` 的目标
        // 对账只看 `tracks` 里有没有这条轨）。没有泳道时也放一条空轨，保证目标对账
        // 与真实工程同解。
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
        #[allow(clippy::cast_precision_loss)]
        let mut smoother = ParamSmoother::with_default_time(sample_rate as f32);
        // 首块的曲线值就是起点：不从这里“淡入”，否则 tick 0 处的自动化值会被
        // 5 ms 的上升沿改写（那是监听侧才需要的防咔嗒行为；离线母带从 0 帧开始，
        // 曲线在 0 帧的值就是作者写的值）。
        if !identity && let Ok(Some(value)) = evaluator.automation_value_at(&target, 0) {
            let ratio = automation_ratio(value, static_gain);
            smoother.snap_to(ratio);
        }
        Self {
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
        }
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

/// R55 换域：`db_to_linear(自动化 dB) / db_to_linear(静态 dB)`。
///
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

    use yeban_model::{AutomationLane, AutomationPoint, AutomationWriteMode, CurveType};

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
}
