//! 75 音分混合移调路由：渲染路由框架（P0-1，vocaledit Phase 2 第二项）。
//!
//! 本模块实现 Phase 2 渲染链的路由层（跟进清单 P0-1、开发计划 §3 第 2 项）：
//!
//! * **路由单位是 chunk**：块内任一有声帧 `|偏差| ≥ 阈值` 则整 chunk
//!   走神经声码器重合成，否则走 DSP 移调（P1-1
//!   [`crate::vocal_pitch_shift::PitchShifter`]）。路由决策按
//!   [`chunk_needs_vocoder`] 语义（跟进清单 P0-1 所述：任一有声帧
//!   超阈则整 chunk 走神经；clean-room 重写，只按跟进清单的算法
//!   描述实现，未打开、未对照、未粘贴任何 OpenTune 代码）。
//! * **阈值是可配置参数**：[`PitchRouteConfig::vocoder_threshold_cents`]，
//!   默认值钉死 [`DEFAULT_VOCODER_THRESHOLD_CENTS`]（75.0 音分），
//!   不得写死后无法复核；配置须 [`PitchRouteConfig::validate`]。
//! * **先 timemap 对时，再按偏差路由**：本模块是接在 Phase 1 warped
//!   钩子（`AudioClipSource::warped`／`AudioPcmCache::warped`）之后的
//!   第二层决策——调用方先用
//!   [`crate::vocal_timemap::render_continuous_timemap`] 对齐时间，
//!   再把 warped PCM 交给 [`HybridPitchRouter::route`]（或
//!   [`route_warped_audio`]）修正音高。时间门检（R、核速率、钳制）
//!   仍在前，不受路由影响；路由只决定“怎么修音高”，不改“对不对得
//!   上时间”。
//! * **收据复用 P1-1 类型**：路由决策写进
//!   [`crate::vocal_pitch_shift::PitchShiftReceipt`]，每 chunk 标记路由
//!   （[`crate::vocal_pitch_shift::ChunkRoute`]，含 `NeuralVocoder`）、
//!   施加音分与最大 `|偏差|`；不另起一套平行收据类型。
//! * **神经腿是接口**：[`NeuralVocoderBackend`] 是最小后端 trait
//!   （输入 warped 后音频 chunk ＋ 目标 F0 轨 ＋ 采样率；输出同长
//!   重合成音频 chunk）。本项是**接口版**：默认后端是
//!   [`UnconfiguredVocoder`]，权重未配置时返回硬错误、渲染大声失败，
//!   绝不静默回落干声。最小 HiFi-GAN ONNX 推理后端**不在本项范围**
//!   内（见下“范围决定”）。
//! * **默认关闭／默认等价 Phase 1 路**：[`PitchRouteConfig::enabled`]
//!   默认 `false`；关闭时 [`HybridPitchRouter::route`] 原样返回 warped
//!   PCM；偏差全为 0 时输出与 Phase 1 路逐位相同（见测试）。
//!
//! # 离线限定
//!
//! 本模块只用于离线渲染，绝不进入实时回调（与 `vocal_timemap`、
//! `vocal_pitch_shift` 同款离线限定）：`route` 分配输出与收据；
//! 神经推理只走离线渲染 stage，RT 回调内绝对禁止。
//!
//! # 范围决定：神经腿接口版（协调员已定）
//!
//! 本项**不含**最小 HiFi-GAN ONNX 推理后端。实测依据：夜半全仓无任何
//! ONNX 运行时或声码器依赖（`Cargo.toml` 零命中，`yeban-dsp`／
//! `yeban-mcp`／`yeban-app` 源码零提及）；引入 ONNX 运行时＋模型加载
//! ＋mel 提取是另一套设计（运行时选型 ort/tract、模型格式、运行时
//! crate 许可 vetting）与另一套验收面，须另立项。声码器权重走用户
//! 自行下载加路径配置（Wang 2026-11-07 08:22 定），不进仓库
//! （`>10 MB` 未注册二进制禁入，见仓库 AGENTS.md §2 第 9 条）。
//! 声码器后端作为下一项交接，见 `REPORT-p0-1.md`。
//!
//! # Clean-room
//!
//! 本模块只按跟进清单 P0-1／P1-1 的算法描述重写（75 音分阈值、
//! chunk 单位、任一有声帧超阈整 chunk 走神经、收据逐 chunk 标记），
//! 未打开、未对照、未粘贴任何 OpenTune 代码。OpenTune 本体
//! AGPL-3.0，禁入 MIT 仓库。

use crate::vocal_pitch_shift::{
    ChunkRoute, DEFAULT_CHUNK_SAMPLES, MAX_SHIFT_CENTS, PitchShiftConfig, PitchShiftReceipt,
    PitchShifter,
};
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// 默认路由阈值（音分）：块内任一有声帧 `|偏差| ≥ 75.0` 则整 chunk 走神经腿。
pub const DEFAULT_VOCODER_THRESHOLD_CENTS: f32 = 75.0;
/// 路由阈值上限（音分）：与移调器最大移调量同口径。
pub const MAX_VOCODER_THRESHOLD_CENTS: f32 = MAX_SHIFT_CENTS;

/// 声码器权重配置：只含可选权重路径，不含权重本身。
///
/// 权重由用户自行下载（Wang 2026-11-07 08:22 定），路径配在这里；
/// 权重不得进仓库。`None` 表示未配置——此时任何需要神经腿的 chunk
/// 都会使渲染以 [`VocoderError::NotConfigured`] 大声失败。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VocoderConfig {
    /// 声码器权重文件路径（用户自行下载）。`None` 表示未配置。
    pub weights_path: Option<PathBuf>,
}

impl VocoderConfig {
    /// 校验：已给的路径不得是空路径。
    ///
    /// # Panics
    ///
    /// `weights_path` 为 `Some` 且为空路径时 panic。
    pub fn validate(&self) {
        if let Some(path) = &self.weights_path {
            assert!(
                !path.as_os_str().is_empty(),
                "vocoder weights_path must not be empty"
            );
        }
    }
}

/// 混合移调路由配置：全部阈值与开关均可配置，默认值钉死（见各字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct PitchRouteConfig {
    /// 路由总开关。默认 `false`（关闭＝Phase 1 路，不修音高）。
    pub enabled: bool,
    /// 路由阈值（音分）：块内任一有声帧 `|偏差| ≥ 此值` 则整 chunk
    /// 走神经腿。默认 [`DEFAULT_VOCODER_THRESHOLD_CENTS`]（75.0）。
    pub vocoder_threshold_cents: f32,
    /// chunk 长度（采样数）：路由与收据的单位。默认
    /// [`DEFAULT_CHUNK_SAMPLES`]（480，10 ms @ 48 kHz）。
    pub chunk_samples: usize,
    /// 声码器权重配置（可选路径）。
    pub vocoder: VocoderConfig,
}

impl Default for PitchRouteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vocoder_threshold_cents: DEFAULT_VOCODER_THRESHOLD_CENTS,
            chunk_samples: DEFAULT_CHUNK_SAMPLES,
            vocoder: VocoderConfig::default(),
        }
    }
}

impl PitchRouteConfig {
    /// 校验全部字段。不合法时 panic（构造期错误，不应进入渲染）。
    ///
    /// # Panics
    ///
    /// `vocoder_threshold_cents` 不在 `(0, 1200]` 内、`chunk_samples == 0`
    /// 或权重配置不合法（见 [`VocoderConfig::validate`]）时 panic。
    pub fn validate(&self) {
        assert!(
            self.vocoder_threshold_cents > 0.0
                && self.vocoder_threshold_cents <= MAX_VOCODER_THRESHOLD_CENTS,
            "vocoder_threshold_cents {} must be in (0, {}]",
            self.vocoder_threshold_cents,
            MAX_VOCODER_THRESHOLD_CENTS
        );
        assert!(self.chunk_samples > 0, "chunk_samples must be > 0");
        self.vocoder.validate();
    }
}

/// 神经声码器后端错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VocoderError {
    /// 声码器权重未配置：渲染不得静默回落干声，必须大声失败。
    NotConfigured,
    /// 后端自身失败（模型加载、推理、输出长度不符等），附原因。
    Backend(String),
}

impl fmt::Display for VocoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VocoderError::NotConfigured => write!(
                f,
                "neural vocoder is not configured: set PitchRouteConfig.vocoder.weights_path to a user-downloaded weights file (weights are never bundled in the repo)"
            ),
            VocoderError::Backend(reason) => write!(f, "neural vocoder backend failed: {reason}"),
        }
    }
}

impl Error for VocoderError {}

/// 路由错误：神经腿失败，或调用方输入不合法。
#[derive(Debug)]
pub enum RouteError {
    /// 神经声码器后端失败（含未配置）。
    Vocoder(VocoderError),
    /// 调用方输入不合法（帧长为 0、chunk 不是帧的整数倍等）。
    InvalidInput(&'static str),
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteError::Vocoder(err) => write!(f, "{err}"),
            RouteError::InvalidInput(msg) => write!(f, "invalid router input: {msg}"),
        }
    }
}

impl Error for RouteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            RouteError::Vocoder(err) => Some(err),
            RouteError::InvalidInput(_) => None,
        }
    }
}

impl From<VocoderError> for RouteError {
    fn from(err: VocoderError) -> Self {
        RouteError::Vocoder(err)
    }
}

/// 神经声码器后端 trait（最小接口，P0-1 接口版）。
///
/// 输入：warped 后音频 chunk（单声道 `f32`）＋ 目标 F0 轨（Hz，
/// 与 chunk 内帧一一对应）＋ 采样率；输出：同长重合成音频 chunk。
/// 实现只走离线渲染 stage，绝不进入实时回调；ONNX 推理后端是
/// 下一项（见模块文档“范围决定”），本项只留此接口。
pub trait NeuralVocoderBackend {
    /// 重合成一个 chunk。返回的 chunk 长度必须等于输入 chunk 长度；
    /// 长度不符时路由层以 [`VocoderError::Backend`] 大声失败。
    fn resynthesize(
        &mut self,
        warped_chunk: &[f32],
        target_f0_hz: &[f32],
        sample_rate: f32,
    ) -> Result<Vec<f32>, VocoderError>;
}

/// 未配置的神经后端：任何调用都返回 [`VocoderError::NotConfigured`]。
///
/// 这是默认后端——权重未配置时渲染大声失败，绝不静默回落干声。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnconfiguredVocoder;

impl NeuralVocoderBackend for UnconfiguredVocoder {
    fn resynthesize(
        &mut self,
        _warped_chunk: &[f32],
        _target_f0_hz: &[f32],
        _sample_rate: f32,
    ) -> Result<Vec<f32>, VocoderError> {
        Err(VocoderError::NotConfigured)
    }
}

/// 一个 chunk 是否走神经腿（任一有声帧超阈则整 chunk 走神经的语义）。
///
/// `frame_deviations_cents` 与 `frame_voiced` 是 chunk 内各帧的偏差
/// （音分）与有声标志（长度不等时按较短者计）。任一**有声**帧
/// `|偏差| ≥ threshold_cents` 即返回 `true`（整 chunk 走神经）；
/// 阈值含边界（`75.0` 音分走神经，`74.9` 音分走 DSP）。
pub fn chunk_needs_vocoder(
    frame_deviations_cents: &[f32],
    frame_voiced: &[bool],
    threshold_cents: f32,
) -> bool {
    frame_deviations_cents
        .iter()
        .zip(frame_voiced.iter())
        .any(|(dev, voiced)| *voiced && dev.abs() >= threshold_cents)
}

/// 75 音分混合移调路由器：DSP 腿（P1-1 移调器）＋ 神经腿（后端 trait）。
///
/// 路由单位是 chunk；先 timemap 对时（调用方在前），再按偏差路由
/// 修正音高。只用于离线渲染。
pub struct HybridPitchRouter {
    sample_rate: f32,
    config: PitchRouteConfig,
    shifter: PitchShifter,
    backend: Box<dyn NeuralVocoderBackend>,
}

impl HybridPitchRouter {
    /// 构造路由器。两份配置都会校验（见
    /// [`PitchRouteConfig::validate`] 与
    /// [`PitchShiftConfig::validate`]）。
    ///
    /// # Panics
    ///
    /// 配置不合法或采样率低于 [`crate::MIN_SAMPLE_RATE`] 时 panic。
    pub fn new(
        sample_rate: f32,
        config: PitchRouteConfig,
        shift_config: PitchShiftConfig,
        backend: Box<dyn NeuralVocoderBackend>,
    ) -> Self {
        config.validate();
        assert!(
            sample_rate >= crate::MIN_SAMPLE_RATE,
            "sample_rate {sample_rate} is below {}",
            crate::MIN_SAMPLE_RATE
        );
        Self {
            sample_rate,
            config,
            shifter: PitchShifter::new(sample_rate, shift_config),
            backend,
        }
    }

    /// 用未配置后端构造路由器（权重未配置时神经腿大声失败）。
    pub fn with_unconfigured_backend(
        sample_rate: f32,
        config: PitchRouteConfig,
        shift_config: PitchShiftConfig,
    ) -> Self {
        Self::new(
            sample_rate,
            config,
            shift_config,
            Box::new(UnconfiguredVocoder),
        )
    }

    /// 当前路由配置。
    pub fn route_config(&self) -> &PitchRouteConfig {
        &self.config
    }

    /// 路由一整段 warped 后单声道音频。
    ///
    /// * `warped`：Phase 1 timemap 对时后的 PCM（单声道 `f32`）。
    /// * `deviations_cents`：每帧的音高偏差（音分，正值表示唱高了、
    ///   须往低修）；一帧为 `frame_samples` 个采样。
    /// * `voiced`：每帧的检测器 valid 标志（浊音为真）。
    /// * `target_f0_hz`：每帧的目标 F0（Hz，神经腿用；DSP 腿不用）。
    ///   三个 per-frame 切片长度不足按缺失帧计（偏差 0、清音、F0 0），
    ///   多余忽略。
    /// * `frame_samples`：每帧采样数；`chunk_samples` 须是它的整数倍。
    ///
    /// 返回 `(输出采样, 收据)`：输出长度恒等于输入长度；收据复用
    /// [`PitchShiftReceipt`]，每 chunk 标记路由、施加音分与最大
    /// `|偏差|`。路由关闭时原样返回输入（收据全 `Passthrough`）；
    /// 神经腿失败（含未配置）时返回 [`RouteError`]，绝不静默回落。
    /// 本函数只用于离线渲染。
    pub fn route(
        &mut self,
        warped: &[f32],
        deviations_cents: &[f32],
        voiced: &[bool],
        target_f0_hz: &[f32],
        frame_samples: usize,
    ) -> Result<(Vec<f32>, PitchShiftReceipt), RouteError> {
        if frame_samples == 0 {
            return Err(RouteError::InvalidInput("frame_samples must be > 0"));
        }
        let chunk = self.config.chunk_samples;
        if !chunk.is_multiple_of(frame_samples) {
            return Err(RouteError::InvalidInput(
                "chunk_samples must be a multiple of frame_samples",
            ));
        }
        let frames_per_chunk = chunk / frame_samples;
        let n_chunks = warped.len().div_ceil(chunk);
        let mut receipt = PitchShiftReceipt::default();
        if warped.is_empty() {
            return Ok((Vec::new(), receipt));
        }
        // 每 chunk 的帧统计：是否走神经、DSP 修正量（有声帧偏差均值）、
        // 最大 |偏差|、是否有声。
        let mut needs_vocoder = Vec::with_capacity(n_chunks);
        let mut correction = Vec::with_capacity(n_chunks);
        let mut max_abs = Vec::with_capacity(n_chunks);
        let mut voiced_any = Vec::with_capacity(n_chunks);
        for c in 0..n_chunks {
            let frame_start = c * frames_per_chunk;
            let frame_end = ((c + 1) * frames_per_chunk).min(deviations_cents.len());
            let devs = if frame_start < frame_end {
                &deviations_cents[frame_start..frame_end]
            } else {
                &[]
            };
            let vcs = if frame_start < voiced.len() {
                &voiced[frame_start..voiced.len().min((c + 1) * frames_per_chunk)]
            } else {
                &[]
            };
            let mut sum = 0.0f32;
            let mut count = 0usize;
            let mut max = 0.0f32;
            for (dev, v) in devs.iter().zip(vcs.iter()) {
                if *v {
                    sum += *dev;
                    count += 1;
                    if dev.abs() > max {
                        max = dev.abs();
                    }
                }
            }
            needs_vocoder.push(chunk_needs_vocoder(
                devs,
                vcs,
                self.config.vocoder_threshold_cents,
            ));
            correction.push(if count > 0 { sum / count as f32 } else { 0.0 });
            max_abs.push(max);
            voiced_any.push(count > 0);
        }
        if !self.config.enabled {
            // 路由关闭＝Phase 1 路：原样返回，收据仍带偏差分布。
            for c in 0..n_chunks {
                receipt.chunks.push(crate::vocal_pitch_shift::ChunkReceipt {
                    chunk_index: c,
                    route: ChunkRoute::Passthrough,
                    cents_applied: 0.0,
                    max_abs_deviation_cents: max_abs[c],
                    voiced: voiced_any[c],
                    period_samples: None,
                    periods_inserted: 0,
                    periods_deleted: 0,
                    transition_fade: false,
                });
            }
            return Ok((warped.to_vec(), receipt));
        }
        // 神经腿先行：任一神经 chunk 失败即大声失败，不跑 DSP 腿。
        let mut neural_out: Vec<Option<Vec<f32>>> = Vec::with_capacity(n_chunks);
        for (c, needs) in needs_vocoder.iter().enumerate() {
            if !*needs {
                neural_out.push(None);
                continue;
            }
            let start = c * chunk;
            let end = (start + chunk).min(warped.len());
            let frame_start = c * frames_per_chunk;
            let mut f0 = vec![0.0f32; frames_per_chunk];
            for (i, slot) in f0.iter_mut().enumerate() {
                if let Some(v) = target_f0_hz.get(frame_start + i) {
                    *slot = *v;
                }
            }
            let resynth = self
                .backend
                .resynthesize(&warped[start..end], &f0, self.sample_rate)?;
            if resynth.len() != end - start {
                return Err(RouteError::Vocoder(VocoderError::Backend(format!(
                    "vocoder output length {} != chunk length {}",
                    resynth.len(),
                    end - start
                ))));
            }
            neural_out.push(Some(resynth));
        }
        // DSP 腿：整段一次移调表调用（读指针／周期网格连续），神经
        // chunk 位置移调量为 0（输出随后被神经结果覆盖）。
        let schedule: Vec<f32> = (0..n_chunks)
            .map(|c| if needs_vocoder[c] { 0.0 } else { correction[c] })
            .collect();
        let (mut output, dsp_receipt) =
            self.shifter
                .process_with_shift_schedule(warped, &voiced_any, &schedule);
        // 拼合神经 chunk，覆写收据为最终路由。
        let mut neural_count = 0u32;
        let mut inserted_total = 0u32;
        let mut deleted_total = 0u32;
        for c in 0..n_chunks {
            let start = c * chunk;
            let end = (start + chunk).min(warped.len());
            let mut chunk_receipt = dsp_receipt.chunks[c].clone();
            chunk_receipt.chunk_index = c;
            chunk_receipt.max_abs_deviation_cents = max_abs[c];
            chunk_receipt.voiced = voiced_any[c];
            if needs_vocoder[c] {
                let resynth = neural_out[c].take().expect("neural output missing");
                output[start..end].copy_from_slice(&resynth);
                chunk_receipt.route = ChunkRoute::NeuralVocoder;
                chunk_receipt.cents_applied = correction[c];
                chunk_receipt.period_samples = None;
                chunk_receipt.periods_inserted = 0;
                chunk_receipt.periods_deleted = 0;
                chunk_receipt.transition_fade = false;
                neural_count += 1;
            } else {
                if correction[c].abs() < f32::EPSILON {
                    chunk_receipt.route = ChunkRoute::Passthrough;
                    chunk_receipt.cents_applied = 0.0;
                    chunk_receipt.period_samples = None;
                }
                inserted_total += chunk_receipt.periods_inserted;
                deleted_total += chunk_receipt.periods_deleted;
            }
            receipt.chunks.push(chunk_receipt);
        }
        receipt.periods_inserted = inserted_total;
        receipt.periods_deleted = deleted_total;
        receipt.neural_chunks = neural_count;
        Ok((output, receipt))
    }
}

/// warped 后第二层决策入口：先 timemap 对时，再按偏差路由修正音高。
///
/// 调用方在 [`crate::vocal_timemap::render_continuous_timemap`] 之后调
/// 用本函数（或直接用 [`HybridPitchRouter::route`]），把路由后的 PCM
/// 交给 warped 钩子（`AudioClipSource::warped`／`AudioPcmCache::warped`）
/// 的调用方；不调用本函数时渲染路径与 Phase 1 完全相同。
/// 神经腿失败（含未配置）时返回 [`RouteError`]，绝不静默回落。
/// 只用于离线渲染。
#[allow(clippy::too_many_arguments)]
pub fn route_warped_audio(
    warped: &[f32],
    sample_rate: f32,
    deviations_cents: &[f32],
    voiced: &[bool],
    target_f0_hz: &[f32],
    frame_samples: usize,
    route_config: PitchRouteConfig,
    shift_config: PitchShiftConfig,
    backend: Box<dyn NeuralVocoderBackend>,
) -> Result<(Vec<f32>, PitchShiftReceipt), RouteError> {
    let mut router = HybridPitchRouter::new(sample_rate, route_config, shift_config, backend);
    router.route(
        warped,
        deviations_cents,
        voiced,
        target_f0_hz,
        frame_samples,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocal_pitch_shift::estimate_period;
    use libm::{log2, sinf};
    use std::cell::RefCell;
    use std::f32::consts::PI;
    use std::rc::Rc;

    const SR: f32 = 48_000.0;
    const CHUNK: usize = 480;

    /// 相位连续的纯音（`phase` 为进出相位，弧度）。
    fn tone(freq: f32, len: usize, phase: &mut f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(0.8 * sinf(*phase));
            *phase += 2.0 * PI * freq / SR;
            if *phase >= 2.0 * PI {
                *phase -= 2.0 * PI;
            }
        }
        out
    }

    /// 音分误差：实测 F0 相对目标 F0。
    fn cents_error(measured: f32, target: f32) -> f32 {
        1200.0 * log2(f64::from(measured) / f64::from(target)) as f32
    }

    /// 假神经后端：只记录被调用，不需真实权重；输出整 chunk 标记值。
    #[derive(Debug, Default)]
    struct FakeVocoderState {
        calls: usize,
        chunk_lens: Vec<usize>,
        f0_lens: Vec<usize>,
    }

    #[derive(Debug, Clone)]
    struct FakeVocoder {
        state: Rc<RefCell<FakeVocoderState>>,
        marker: f32,
    }

    impl NeuralVocoderBackend for FakeVocoder {
        fn resynthesize(
            &mut self,
            warped_chunk: &[f32],
            target_f0_hz: &[f32],
            _sample_rate: f32,
        ) -> Result<Vec<f32>, VocoderError> {
            let mut state = self.state.borrow_mut();
            state.calls += 1;
            state.chunk_lens.push(warped_chunk.len());
            state.f0_lens.push(target_f0_hz.len());
            Ok(vec![self.marker; warped_chunk.len()])
        }
    }

    /// 440 Hz 纯音 take：`n_chunks` 个 chunk，每 chunk 480 采样。
    fn take_440(n_chunks: usize) -> Vec<f32> {
        let mut phase = 0.0f32;
        tone(440.0, n_chunks * CHUNK, &mut phase)
    }

    /// 带假后端的路由器（阈值可配）与共享的调用记录。
    fn router_with_fake(
        enabled: bool,
        threshold_cents: f32,
    ) -> (HybridPitchRouter, Rc<RefCell<FakeVocoderState>>) {
        let state = Rc::new(RefCell::new(FakeVocoderState::default()));
        let fake = FakeVocoder {
            state: Rc::clone(&state),
            marker: 0.5,
        };
        let config = PitchRouteConfig {
            enabled,
            vocoder_threshold_cents: threshold_cents,
            ..PitchRouteConfig::default()
        };
        let router =
            HybridPitchRouter::new(SR, config, PitchShiftConfig::default(), Box::new(fake));
        (router, state)
    }

    /// 每 chunk 路由列表（便于整表断言）。
    fn routes(receipt: &PitchShiftReceipt) -> Vec<ChunkRoute> {
        receipt.chunks.iter().map(|c| c.route).collect()
    }

    #[test]
    fn defaults_are_pinned() {
        let cfg = PitchRouteConfig::default();
        assert!(!cfg.enabled, "routing must default to off (Phase 1 path)");
        assert_eq!(cfg.vocoder_threshold_cents.to_bits(), 75.0f32.to_bits());
        assert_eq!(cfg.chunk_samples, 480);
        assert_eq!(cfg.vocoder.weights_path, None);
        let voc = VocoderConfig::default();
        assert_eq!(voc.weights_path, None);
    }

    #[test]
    #[should_panic(expected = "vocoder_threshold_cents")]
    fn zero_threshold_is_rejected() {
        PitchRouteConfig {
            vocoder_threshold_cents: 0.0,
            ..PitchRouteConfig::default()
        }
        .validate();
    }

    #[test]
    #[should_panic(expected = "vocoder_threshold_cents")]
    fn over_max_threshold_is_rejected() {
        PitchRouteConfig {
            vocoder_threshold_cents: 1200.1,
            ..PitchRouteConfig::default()
        }
        .validate();
    }

    #[test]
    #[should_panic(expected = "chunk_samples must be > 0")]
    fn zero_chunk_is_rejected() {
        PitchRouteConfig {
            chunk_samples: 0,
            ..PitchRouteConfig::default()
        }
        .validate();
    }

    #[test]
    #[should_panic(expected = "weights_path must not be empty")]
    fn empty_weights_path_is_rejected() {
        VocoderConfig {
            weights_path: Some(PathBuf::from("")),
        }
        .validate();
    }

    #[test]
    fn chunk_needs_vocoder_boundary() {
        let threshold = 75.0f32;
        // 边界含：75.0 走神经，74.9 走 DSP。
        assert!(!chunk_needs_vocoder(&[74.9], &[true], threshold));
        assert!(chunk_needs_vocoder(&[75.0], &[true], threshold));
        assert!(chunk_needs_vocoder(&[-75.0], &[true], threshold));
        // 清音帧不触发：100 音分清音仍走 DSP。
        assert!(!chunk_needs_vocoder(&[100.0], &[false], threshold));
        // 块内任一有声帧超阈即整 chunk 走神经。
        assert!(!chunk_needs_vocoder(
            &[10.0, 80.0],
            &[true, false],
            threshold
        ));
        assert!(chunk_needs_vocoder(&[10.0, 80.0], &[true, true], threshold));
        assert!(!chunk_needs_vocoder(
            &[10.0, 74.0, 75.0],
            &[true, true, false],
            threshold
        ));
        assert!(chunk_needs_vocoder(
            &[10.0, 74.0, 75.0],
            &[true, true, true],
            threshold
        ));
        assert!(!chunk_needs_vocoder(&[], &[], threshold));
    }

    #[test]
    fn six_grade_routing_matches_threshold() {
        // 六档（25／50／74／75／100／200 音分）：74 走 DSP、75 走神经。
        let grades = [25.0f32, 50.0, 74.0, 75.0, 100.0, 200.0];
        for grade in grades {
            let input = take_440(10);
            let (mut router, state) = router_with_fake(true, 75.0);
            let devs = vec![grade; 10];
            let voiced = vec![true; 10];
            let f0 = vec![440.0f32; 10];
            let (output, receipt) = router
                .route(&input, &devs, &voiced, &f0, CHUNK)
                .expect("routing must succeed with a fake backend");
            assert_eq!(output.len(), input.len());
            assert_eq!(receipt.chunks.len(), 10);
            let expect_neural = grade >= 75.0;
            let want = if expect_neural {
                ChunkRoute::NeuralVocoder
            } else {
                ChunkRoute::DspShift
            };
            assert_eq!(
                routes(&receipt),
                vec![want; 10],
                "grade {grade} must route every chunk to {want:?}"
            );
            for chunk in receipt.chunks.iter() {
                assert_eq!(chunk.max_abs_deviation_cents, grade);
                assert_eq!(chunk.cents_applied, grade);
            }
            assert_eq!(
                receipt.neural_chunks,
                if expect_neural { 10 } else { 0 },
                "grade {grade} neural_chunks"
            );
            let calls = state.borrow().calls;
            assert_eq!(
                calls,
                if expect_neural { 10 } else { 0 },
                "grade {grade} backend calls"
            );
            if expect_neural {
                assert_eq!(
                    output,
                    vec![0.5f32; input.len()],
                    "grade {grade} output is the fake vocoder marker"
                );
            } else {
                assert_ne!(output, input, "grade {grade} DSP leg must shift the audio");
            }
            eprintln!("grade {grade}: route {want:?}, backend calls {calls}");
        }
    }

    #[test]
    fn dsp_leg_shifts_f0_toward_target() {
        // DSP 腿（+50 音分）的输出 F0 须落在目标 ±5 音分内（P1-1 移调器
        // 的验收线，路由层只复核一路）。
        let input = take_440(10);
        let (mut router, _) = router_with_fake(true, 75.0);
        let devs = vec![50.0f32; 10];
        let voiced = vec![true; 10];
        let f0 = vec![440.0f32; 10];
        let (output, _) = router
            .route(&input, &devs, &voiced, &f0, CHUNK)
            .expect("routing must succeed");
        let period = estimate_period(&output, SR).expect("period must be found");
        let measured = SR / period;
        let target = 440.0 * 2.0f32.powf(50.0 / 1200.0);
        let err = cents_error(measured, target);
        eprintln!("dsp leg +50 cents: measured {measured:.2} Hz ({err:.2} cents)");
        assert!(
            err.abs() <= 5.0,
            "DSP leg F0 error {err:.2} cents exceeds +/-5"
        );
    }

    #[test]
    fn unconfigured_backend_fails_loud() {
        // 75 音分以上须走神经腿：未配置后端必须大声失败。
        let input = take_440(4);
        let config = PitchRouteConfig {
            enabled: true,
            ..PitchRouteConfig::default()
        };
        let mut router =
            HybridPitchRouter::with_unconfigured_backend(SR, config, PitchShiftConfig::default());
        let devs = vec![100.0f32; 4];
        let voiced = vec![true; 4];
        let f0 = vec![440.0f32; 4];
        let err = router
            .route(&input, &devs, &voiced, &f0, CHUNK)
            .expect_err("unconfigured vocoder must fail loudly");
        assert!(
            matches!(err, RouteError::Vocoder(VocoderError::NotConfigured)),
            "error must be NotConfigured, got {err}"
        );
        // 75 音分以下走 DSP 腿：未配置后端也不影响，须成功。
        let devs_dsp = vec![50.0f32; 4];
        let (output, receipt) = router
            .route(&input, &devs_dsp, &voiced, &f0, CHUNK)
            .expect("DSP-only take must succeed without a vocoder");
        assert_eq!(output.len(), input.len());
        assert_eq!(receipt.neural_chunks, 0);
        assert_eq!(routes(&receipt), vec![ChunkRoute::DspShift; 4]);
    }

    #[test]
    fn disabled_router_is_passthrough() {
        // 路由关闭＝Phase 1 路：哪怕偏差 200 音分也原样返回，后端零调用。
        let input = take_440(10);
        let (mut router, state) = router_with_fake(false, 75.0);
        let devs = vec![200.0f32; 10];
        let voiced = vec![true; 10];
        let f0 = vec![440.0f32; 10];
        let (output, receipt) = router
            .route(&input, &devs, &voiced, &f0, CHUNK)
            .expect("disabled routing must succeed");
        assert_eq!(output, input, "disabled routing must be bit-identical");
        assert_eq!(receipt.chunks.len(), 10);
        assert_eq!(routes(&receipt), vec![ChunkRoute::Passthrough; 10]);
        assert_eq!(receipt.neural_chunks, 0);
        assert_eq!(state.borrow().calls, 0);
        for chunk in receipt.chunks.iter() {
            assert_eq!(chunk.max_abs_deviation_cents, 200.0);
            assert_eq!(chunk.cents_applied, 0.0);
        }
    }

    #[test]
    fn zero_deviation_is_bit_identical_to_phase1() {
        // 偏差 0 的 take 与 Phase 1 路逐位相同（Phase 1 路不修音高）。
        let input = take_440(10);
        let config = PitchRouteConfig {
            enabled: true,
            ..PitchRouteConfig::default()
        };
        let mut router =
            HybridPitchRouter::with_unconfigured_backend(SR, config, PitchShiftConfig::default());
        let devs = vec![0.0f32; 10];
        let voiced = vec![true; 10];
        let f0 = vec![440.0f32; 10];
        let (output, receipt) = router
            .route(&input, &devs, &voiced, &f0, CHUNK)
            .expect("zero-deviation routing must succeed");
        assert_eq!(output, input, "zero deviation must be bit-identical");
        assert_eq!(routes(&receipt), vec![ChunkRoute::Passthrough; 10]);
        assert_eq!(receipt.neural_chunks, 0);
        assert_eq!(receipt.periods_inserted, 0);
        assert_eq!(receipt.periods_deleted, 0);
    }

    #[test]
    fn mixed_take_routes_per_chunk() {
        // 混合 take：偏差 [10, 74, 75, 200] → [DSP, DSP, 神经, 神经]，
        // 收据每 chunk 标记齐全，神经 chunk 由后端输出拼合。
        let input = take_440(4);
        let (mut router, state) = router_with_fake(true, 75.0);
        let devs = vec![10.0f32, 74.0, 75.0, 200.0];
        let voiced = vec![true; 4];
        let f0 = vec![440.0f32; 4];
        let (output, receipt) = router
            .route(&input, &devs, &voiced, &f0, CHUNK)
            .expect("routing must succeed");
        assert_eq!(output.len(), input.len());
        assert_eq!(receipt.chunks.len(), 4);
        assert_eq!(
            routes(&receipt),
            vec![
                ChunkRoute::DspShift,
                ChunkRoute::DspShift,
                ChunkRoute::NeuralVocoder,
                ChunkRoute::NeuralVocoder,
            ]
        );
        let max_abs: Vec<f32> = receipt
            .chunks
            .iter()
            .map(|c| c.max_abs_deviation_cents)
            .collect();
        assert_eq!(max_abs, vec![10.0, 74.0, 75.0, 200.0]);
        let applied: Vec<f32> = receipt.chunks.iter().map(|c| c.cents_applied).collect();
        assert_eq!(applied, vec![10.0, 74.0, 75.0, 200.0]);
        assert_eq!(receipt.neural_chunks, 2);
        assert_eq!(state.borrow().calls, 2);
        // 神经 chunk 是后端标记值，DSP chunk 不是。
        assert_eq!(output[2 * CHUNK..3 * CHUNK], vec![0.5f32; CHUNK][..]);
        assert_eq!(output[3 * CHUNK..4 * CHUNK], vec![0.5f32; CHUNK][..]);
        assert_ne!(output[..2 * CHUNK], vec![0.5f32; 2 * CHUNK][..]);
        // 总计等于各 chunk 之和。
        let mut inserted = 0u32;
        let mut deleted = 0u32;
        for chunk in receipt.chunks.iter() {
            inserted += chunk.periods_inserted;
            deleted += chunk.periods_deleted;
        }
        assert_eq!(receipt.periods_inserted, inserted);
        assert_eq!(receipt.periods_deleted, deleted);
    }

    #[test]
    fn route_warped_audio_entry_point() {
        // 第二层决策入口：启用时路由，关闭时逐位相同。
        let input = take_440(4);
        let state = Rc::new(RefCell::new(FakeVocoderState::default()));
        let fake = FakeVocoder {
            state: Rc::clone(&state),
            marker: 0.5,
        };
        let devs = vec![50.0f32; 4];
        let voiced = vec![true; 4];
        let f0 = vec![440.0f32; 4];
        let config = PitchRouteConfig {
            enabled: true,
            ..PitchRouteConfig::default()
        };
        let (output, receipt) = route_warped_audio(
            &input,
            SR,
            &devs,
            &voiced,
            &f0,
            CHUNK,
            config,
            PitchShiftConfig::default(),
            Box::new(fake),
        )
        .expect("entry point must succeed");
        assert_eq!(output.len(), input.len());
        assert_eq!(routes(&receipt), vec![ChunkRoute::DspShift; 4]);
        assert_eq!(state.borrow().calls, 0);
        let off = PitchRouteConfig::default();
        let (plain, _) = route_warped_audio(
            &input,
            SR,
            &devs,
            &voiced,
            &f0,
            CHUNK,
            off,
            PitchShiftConfig::default(),
            Box::new(UnconfiguredVocoder),
        )
        .expect("disabled entry point must succeed");
        assert_eq!(plain, input, "disabled entry point must be bit-identical");
    }

    #[test]
    fn invalid_frame_size_is_rejected() {
        let input = take_440(2);
        let (mut router, _) = router_with_fake(true, 75.0);
        let devs = vec![50.0f32; 2];
        let voiced = vec![true; 2];
        let f0 = vec![440.0f32; 2];
        let err = router
            .route(&input, &devs, &voiced, &f0, 0)
            .expect_err("frame_samples 0 must be rejected");
        assert!(matches!(err, RouteError::InvalidInput(_)));
        let err = router
            .route(&input, &devs, &voiced, &f0, 320)
            .expect_err("chunk 480 must not accept frame 320");
        assert!(matches!(err, RouteError::InvalidInput(_)));
    }
}
