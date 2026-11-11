//! 自动相位对齐的渲染侧：两轨 solo 渲染 + 互相关测量。
//!
//! 手动相位对齐第一层只给执行机构（轨延时 / Sample Delay / 片段偏移）；
//! 本模块是第二层的渲染入口：把基准轨与目标轨各 solo 渲染一遍，混成单声道，
//! 交给 [`yeban_dsp::phase_align_auto::measure_delay`] 量到达时差。
//!
//! 渲染复用 [`crate::song_overview::render_stems_for_tracks_to_buffers`] 的
//! clone-mute-re-render 招数（只渲两条目标轨，不渲全工程）；测量本身住在
//! `yeban-dsp`，本模块只做混音与错误映射，⛔ 不复制互相关算法。

use yeban_dsp::phase_align_auto::{DelayMeasurement, MeasureDelayError, measure_delay};
use yeban_model::{EntityId, YebanProjectV1};

use crate::engine_source::ArmedSfzInput;
use crate::song_overview::{RenderSettings, SongOverviewError, render_stems_for_tracks_to_buffers};

/// 两轨相位测量的失败原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseAlignAutoError {
    /// 渲染失败（采样率 / 工程无内容 / 武装失败 / 目标轨不是源节点）。
    Render(SongOverviewError),
    /// 测量失败（信号太短 / 任一轨静音 / 搜索窗非法）。
    Measure(MeasureDelayError),
}

impl core::fmt::Display for PhaseAlignAutoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Render(error) => write!(f, "相位测量渲染失败: {error}"),
            Self::Measure(error) => write!(f, "相位测量失败: {error}"),
        }
    }
}

impl std::error::Error for PhaseAlignAutoError {}

impl From<SongOverviewError> for PhaseAlignAutoError {
    fn from(error: SongOverviewError) -> Self {
        Self::Render(error)
    }
}

impl From<MeasureDelayError> for PhaseAlignAutoError {
    fn from(error: MeasureDelayError) -> Self {
        Self::Measure(error)
    }
}

/// 一次两轨相位测量的结果（[`DelayMeasurement`] + 渲染口径）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredStemDelay {
    /// 互相关测量（lag / 相关系数 / 极性 / 置信度 / 分析窗）。
    pub measurement: DelayMeasurement,
    /// 渲染采样率（Hz；恒等于工程 `audio_config.sample_rate`）。
    pub sample_rate_hz: u32,
}

/// 交错立体声混成单声道：`(L + R) / 2`（与总线求和同一口径；测量只关心
/// 到达时差，声相不参与）。
fn mono_mixdown(interleaved_stereo: &[f32]) -> Vec<f32> {
    interleaved_stereo
        .chunks_exact(2)
        .map(|pair| (pair[0] + pair[1]) * 0.5)
        .collect()
}

/// 测两条轨的到达时差：各 solo 渲染一遍 → 单声道混音 → 互相关。
///
/// `reference` 是基准轨（不动的锚），`target` 是被测轨；`max_lag_samples`
/// 是搜索窗（±，采样；调用方保证 ≤ `PHASE_ALIGN_MAX_ABS_SAMPLES`，否则
/// 测出的 lag 写不进轨延时）。
///
/// 两轨相同（`reference == target`）时只渲染一次，结果恒为 lag 0、ρ ≈ 1。
///
/// # Errors
///
/// - 目标轨不是源节点 / 渲染失败 → [`PhaseAlignAutoError::Render`]；
/// - 任一轨静音 / 信号太短 / 搜索窗为 0 → [`PhaseAlignAutoError::Measure`]。
pub fn measure_stem_delay<F>(
    project: &YebanProjectV1,
    settings: &RenderSettings,
    sfz_provider: F,
    reference: EntityId,
    target: EntityId,
    max_lag_samples: usize,
) -> Result<MeasuredStemDelay, PhaseAlignAutoError>
where
    F: FnMut(EntityId, &str) -> ArmedSfzInput,
{
    let wanted: &[EntityId] = if reference == target {
        &[reference]
    } else {
        &[reference, target]
    };
    let stems = render_stems_for_tracks_to_buffers(project, settings, sfz_provider, wanted)?;
    let find_stem = |track: EntityId| {
        stems
            .iter()
            .find(|stem| stem.track == track)
            .expect("render_stems_for_tracks_to_buffers 按请求返回每条轨")
    };
    let reference_mono = mono_mixdown(&find_stem(reference).audio.samples);
    let target_mono = mono_mixdown(&find_stem(target).audio.samples);
    let measurement = measure_delay(&reference_mono, &target_mono, max_lag_samples)?;
    Ok(MeasuredStemDelay {
        measurement,
        sample_rate_hz: project.audio_config.sample_rate.hz(),
    })
}
