//! `yeban_auto_align_tracks` 的**真实 provider**（自动相位对齐，手动层第二层）：
//! 把 `yeban_render::phase_align_auto::measure_stem_delay`（两轨 solo 渲染 →
//! 单声道混音 → `yeban-dsp` 归一化互相关）的输出映射成 MCP 的
//! [`PhaseDelayMeasurement`]。
//!
//! ## 为什么住在这里（`yeban-mcp`，两种形态共用同一份实现）
//!
//! 与 [`crate::song_overview_provider`] 同一条裁决：本模块原住 `yeban-app`，
//! 形态 B（独立 stdio 二进制）注入不到 ⇒ 工具恒回 `providerUnavailable`
//! （清单 `05_yeban.md` #3 / #4）。迁入后两种形态构造同一个 provider；
//! 依赖方向上只用 `yeban-render`（已登记例外那条边）与本 crate 的领域层，
//! ⛔ 不直接依赖 `yeban-dsp` / `yeban-engine`（守卫
//! `check_mcp_dependency_direction.py`）。上下文（工程目录 + CAS 池）的
//! 两种接法见 [`crate::song_overview_provider::ProviderContext`]。
//!
//! ## SFZ 武装复用总览 provider 的预检
//!
//! 渲染前必须先把待摊平的 SFZ 全部武装好（`EngineTimbre::from_project_armed`
//! 的契约是「provider 是可靠的」）。武装逻辑只有一份 ——
//! [`RealSongOverviewProvider::arm_all`](crate::song_overview_provider::RealSongOverviewProvider)，
//! 本 provider 每次测量时构造一个只用于武装的总览 provider 实例调它，
//! ⛔ 不复制第二份磁盘腿 / 内嵌腿实现。任一 SFZ 失败 ⇒ `RENDER_FAILED`
//! （`reason: "samplerUnreadable"`）响亮失败，不渲染。
//!
//! ## 失败映射
//!
//! 渲染 / 测量失败统一报 `RENDER_FAILED` + `reason: "renderFailed"` +
//! `cause`（细节串）—— 抄总览 provider 的先例：`fault_vocabulary` 只认
//! 已登记的 reason，因此不在这里立 `silentTrack` / `signalTooShort`
//! 等新 reason，细节放进 `cause`。

use serde_json::json;

use yeban_model::{EntityId, YebanProjectV1};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::phase_align_auto::measure_stem_delay;
use yeban_render::song_overview::RenderSettings;

use crate::domain::error::Fault;
use crate::domain::phase_align_auto::{
    MeasurementConfidence, PhaseAlignAutoProvider, PhaseDelayMeasurement,
};
use crate::song_overview_provider::{RealSongOverviewProvider, SharedProviderContext};
use crate::tools::ErrorCode;

/// 真实 provider 的自述名（进响应 `data.provider`；stub 的是 `"stub"`）。
pub const PROVIDER_NAME: &str = "real";

/// 自动相位对齐的真实 provider：两轨 solo 渲染 + 互相关测量。
///
/// `Debug` 是 trait 约束的一部分（`Domain` 派生 `Debug`）。
#[derive(Debug)]
pub struct RealPhaseAlignAutoProvider {
    /// 会话上下文（工程目录 + CAS 池；与总览 provider 共用同一份共享单元）。
    context: SharedProviderContext,
}

impl RealPhaseAlignAutoProvider {
    /// 造一个真实 provider（上下文的两种接法见
    /// [`crate::song_overview_provider::ProviderContext`]）。
    #[must_use]
    pub fn new(context: SharedProviderContext) -> Self {
        Self { context }
    }
}

impl PhaseAlignAutoProvider for RealPhaseAlignAutoProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn measure(
        &self,
        project: &YebanProjectV1,
        reference: EntityId,
        target: EntityId,
        max_lag_samples: usize,
    ) -> Result<PhaseDelayMeasurement, Fault> {
        let sample_rate_hz = project.audio_config.sample_rate.hz();
        let settings = RenderSettings::new(sample_rate_hz, 0);
        // 预检先行（见模块文档）：武装复用总览 provider 的唯一实现。
        let armer = RealSongOverviewProvider::new(self.context.clone());
        let armed = armer.arm_all(project, sample_rate_hz as f32)?;
        let measured = measure_stem_delay(
            project,
            &settings,
            |track, raw| {
                armed
                    .get(&track)
                    .cloned()
                    .unwrap_or_else(|| ArmedSfzInput::empty(raw))
            },
            reference,
            target,
            max_lag_samples,
        )
        .map_err(|error| {
            Fault::domain_with_data(
                ErrorCode::RenderFailed,
                format!("相位测量失败: {error}"),
                json!({
                    "reason": "renderFailed",
                    "cause": error.to_string(),
                }),
            )
        })?;
        let m = measured.measurement;
        Ok(PhaseDelayMeasurement {
            lag_samples: m.lag_samples,
            lag_samples_rounded: m.lag_samples_rounded,
            correlation: m.correlation,
            polarity_inverted: m.polarity_inverted,
            confidence: MeasurementConfidence::from_correlation(m.correlation),
            window_start_frame: m.window_start_frame,
            window_frames: m.window_frames,
            sample_rate_hz: measured.sample_rate_hz,
        })
    }
}
