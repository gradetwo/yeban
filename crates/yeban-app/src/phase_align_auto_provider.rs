//! `yeban_auto_align_tracks` 的**真实 provider**（自动相位对齐，手动层第二层）：
//! `yeban-app` 侧的接线，把 `yeban_render::phase_align_auto::measure_stem_delay`
//! （两轨 solo 渲染 → 单声道混音 → `yeban-dsp` 归一化互相关）的输出映射成
//! MCP 的 [`PhaseDelayMeasurement`]。
//!
//! ## 为什么住在这里（不在 `yeban-mcp` 里）
//!
//! 与 [`crate::song_overview_provider`] 同一条纪律：`yeban-mcp` 不依赖
//! `yeban-dsp` / `yeban-render`（守卫 `check_mcp_dependency_direction.py`），
//! app 能同时依赖三者，因此 provider 住在这里，挂载时经
//! `Domain::set_phase_align_auto_provider` 注入
//! （`crate::mcp_mount::session_dispatcher`）。
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
//! `cause`（细节串）—— 抄总览 provider 的先例：`fault_vocabulary` 只扫描
//! `yeban-mcp/src`，app 侧新立的 reason 进不了词表，因此不在 app 侧立
//! `silentTrack` / `signalTooShort` 等新 reason，细节放进 `cause`。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::phase_align_auto::{
    MeasurementConfidence, PhaseAlignAutoProvider, PhaseDelayMeasurement,
};
use yeban_mcp::tools::ErrorCode;
use yeban_model::{AssetHash, EntityId, YebanProjectV1};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::phase_align_auto::measure_stem_delay;
use yeban_render::song_overview::RenderSettings;

use crate::song_overview_provider::RealSongOverviewProvider;

/// 真实 provider 的自述名（进响应 `data.provider`；stub 的是 `"stub"`）。
pub const PROVIDER_NAME: &str = "real";

/// 自动相位对齐的真实 provider：两轨 solo 渲染 + 互相关测量。
///
/// 只在 `in-process-mcp` feature 下编译（见 `crate::phase_align_auto_provider`
/// 的 `cfg` 门）。`Debug` 是 trait 约束的一部分（`Domain` 派生 `Debug`）。
#[derive(Debug)]
pub struct RealPhaseAlignAutoProvider {
    /// 磁盘腿 SFZ 解析的工程目录（`None` = 内存会话 / 无磁盘对应物）。
    project_dir: Option<PathBuf>,
    /// 会话 CAS 池（内嵌腿的 SFZ 文本与样本由此取；挂载时的一份拷贝）。
    assets: Arc<BTreeMap<AssetHash, Vec<u8>>>,
}

impl RealPhaseAlignAutoProvider {
    /// 造一个真实 provider（入参与 [`RealSongOverviewProvider::new`] 同口径）。
    #[must_use]
    pub fn new(project_dir: Option<PathBuf>, assets: Arc<BTreeMap<AssetHash, Vec<u8>>>) -> Self {
        Self {
            project_dir,
            assets,
        }
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
        let armer = RealSongOverviewProvider::new(self.project_dir.clone(), self.assets.clone());
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
