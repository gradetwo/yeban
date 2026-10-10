//! 采样率转换：`rubato` 的 sinc（多相）重采样封装 [ARCH-DSP-002]。
//!
//! ## 为什么是 sinc 而不是 FFT
//!
//! 规范 §10.2 的原话是"高精**多相**重采样滤波"，而 `rubato` 的多相实现就是
//! sinc 插值（`Async::new_sinc`）。`rubato` 5.0.1 里还有同步 FFT 重采样
//! （`Fft`），它更快，但要打开 `fft_resampler` feature（会拉入 `realfft`/`rustfft`）。
//! 本 crate 选择 sinc，理由是：
//!
//! 1. **规范措辞**点名"多相/sinc"，不是 FFT；
//! 2. `fft_resampler` 是**额外 feature**，而根 `Cargo.toml` 已把 `rubato` 钉成
//!    `default-features = false` —— 不开 feature 就不需要为依赖图新增任何包；
//! 3. FFT 路径的最后一步在频域做实数到实数变换，其舍入行为与编译期 SIMD 选择耦合，
//!    对 [ARCH-DET-001] 的 L2（跨架构 `< 1e-6`）而言是额外的噪声源。
//!
//! ## 延迟 / 预填充语义（**写死进判据**）
//!
//! `rubato` 的 sinc 重采样器内部有一个"前置延迟"（滤波器需要先填充历史）。本模块
//! 一律走 `Resampler::process_all_into_buffer`，它**按文档把启动延迟裁掉**：
//!
//! > "The processed frames are written to the output buffer, with the initial silence
//! > (caused by the resampler delay) trimmed off."
//!
//! 因此本模块的契约是：
//! - **输出不带前置静音**：`prefill_frames() == 0`，且若输入以直流 `1.0` 开头，
//!   输出的第 0 个样本就已经接近 `1.0`（判据 `no_leading_pad_*`）；
//! - **输出帧数**满足 [`crate::limits::resample_len_contract`]：
//!   `⌊输入帧数 × 输出率 / 输入率⌋`（±0.1% 或 ±8 帧，取大者；短片段再放宽滤波器长度）；
//! - **比例不可调**：[`MAX_RELATIVE_RATIO`] = 1.0，本模块**不**暴露 `set_resample_ratio`。
//!   一次导入只对应一个固定比例，免得"同输入两次导入结果不同"。
//!
//! ## 确定性（[ARCH-DET-001]）
//!
//! - 固定算法（sinc）、固定窗（`BlackmanHarris2`）、固定 `sinc_len`（[`SINC_LEN`]）、
//!   固定分块（[`CHUNK_FRAMES`]）、固定最大相对比例；
//! - 不使用任何随机源，不依赖时钟、线程数或环境变量；
//! - 判据 `same_input_resamples_identically_in_two_threads` 直接跑两次比对**位模式**。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};

use crate::asset::{DecodeFacts, DecodedAsset, PcmFormat};
use crate::duration::Reconciliation;
use crate::error::{DecodeError, DecodeResult};
use crate::limits::{self, LimitViolation, PcmBudget};

/// sinc 滤波器抽头数 —— 与上游构造参数里的字面量 `256` 是同一个被钉死的值。
///
/// 上游参数位宽（`usize` vs `u32`）不确定，因此调用点写字面量、这里留常量供判据引用；
/// 判据 `configuration_is_pinned` 保证两者不会漂移。
pub const SINC_LEN: usize = 256;

/// 内部处理分块（帧）。
///
/// 1024 与 [ARCH-DET-001] 里"固定统一处理块大小（128 采样点）"的精神一致：**固定**才是
/// 确定性的关键，具体数字不是。上游文档推荐的起点就是"1024 这种 2 的幂"。
pub const CHUNK_FRAMES: usize = 1024;

/// 允许在构造后调整比例的最大倍数 —— 固定为 `1.0`。
///
/// 上游要求该值 `>= 1.0`。取 1.0 的语义是"比例从构造那一刻起就是死的"，因此本模块
/// 不存在"被谁在运行中改过比例"的可能，也把内部缓冲需求压到最小。
pub const MAX_RELATIVE_RATIO: f64 = 1.0;

/// 本模块保证的预填充（前置静音）帧数：**0**。
///
/// 见模块文档的延迟语义一节。
pub const PREFILL_FRAMES: u64 = 0;

/// 把一段交织 `f32` 从 `in_rate` 转换到 `out_rate`（使用[默认预算][PcmBudget::default]）。
///
/// 这是给"只做一次转换、不关心预算口径"的调用方（例如 `yeban-mcp` 的
/// `yeban_render_master`）准备的薄入口：它**不是**兼容层 —— 它走的是同一个
/// [`PcmBudget`]，只是填默认值，与 `DecodeOptions::default()` 填默认预算是同一件事。
/// 需要按工程调预算的调用方请用 [`resample_interleaved_with_budget`]。
///
/// 返回值长度满足 [`limits::resample_len_contract`]；`in_rate == out_rate` 时是**恒等**
/// （原样返回，不做任何滤波），因此"同率转换"是零成本的，也天然确定。
///
/// # Errors
///
/// 同 [`resample_interleaved_with_budget`]。
pub fn resample_interleaved(
    samples: &[f32],
    channels: u16,
    in_rate: u32,
    out_rate: u32,
) -> DecodeResult<Vec<f32>> {
    resample_interleaved_with_budget(samples, channels, in_rate, out_rate, &PcmBudget::default())
}

/// 把一段交织 `f32` 从 `in_rate` 转换到 `out_rate`，所有资源上限走 `budget`。
///
/// 为什么重采样也要走预算：上采样会让长度翻倍（48k → 192k 是 4×），输出缓冲是**新**
/// 分配的一份完整 PCM。若这里继续用写死的常量，调用方给 `decode` 的预算就在重采样这一
/// 步被悄悄绕过 —— `HD-24` 改建后所有使用点必须走同一个 [`PcmBudget`]。
///
/// ## 非有限样本（NaN / ±∞）**不被拒绝** —— 这是有意的
///
/// 本函数是纯 DSP 函数，吃的是调用方给的类型化 `&[f32]`，不是不可信字节：它只校验
/// **形状**（`samples.len()` 是 `channels` 的整数倍、三者都在闸门内）。线性滤波对非有限
/// 输入的行为由 IEEE 754 逐位规定，因此本函数保留传播而不是清洗 —— 清洗会静默改变
/// 信号，而"静默改变信号"正是本 crate 要避免的。判据
/// `non_finite_input_propagates_deterministically_and_keeps_the_length_contract` 把这条
/// 语义钉住（恒等路径逐位原样、滤波路径两次调用逐位一致、输出长度与有限输入相同）。
///
/// 不可信字节的边界在 [`crate::decode`]：`decode_*` 会拒绝**含非有限浮点样本的容器**
/// （判据 `a_float_container_with_non_finite_samples_is_refused`），因此本 crate 内部
/// 唯一一条"资产 → 重采样"的路径（[`resample_asset_with_budget`]）拿到的样本必然有限。
///
/// # Errors
///
/// - 采样率为 0 或超出 [`PcmBudget::max_sample_rate`]；
/// - 输出率超过输入率的 [`PcmBudget::max_resample_ratio`] 倍（比例闸门，见 [`limits::check_resample_ratio`]）；
/// - `samples.len()` 不是 `channels` 的整数倍；
/// - 声道数超出 [`PcmBudget::max_channels`]（在任何分配之前判，见
///   `an_over_budget_channel_count_is_refused_before_any_resampling`）；
/// - 输出会超出 [`PcmBudget::max_pcm_bytes`] 或 [`PcmBudget::max_duration_secs`]；
/// - 重采样器构造或处理失败；
/// - 输出帧数落在长度契约之外（这会是一条真正的实现缺陷）。
pub fn resample_interleaved_with_budget(
    samples: &[f32],
    channels: u16,
    in_rate: u32,
    out_rate: u32,
    budget: &PcmBudget,
) -> DecodeResult<Vec<f32>> {
    if channels == 0 {
        return Err(DecodeError::Budget(LimitViolation::ZeroChannels));
    }
    if in_rate == 0 || out_rate == 0 {
        return Err(DecodeError::Budget(LimitViolation::ZeroSampleRate));
    }
    if out_rate > budget.max_sample_rate || in_rate > budget.max_sample_rate {
        return Err(DecodeError::Budget(LimitViolation::SampleRateTooHigh {
            rate: in_rate.max(out_rate),
            limit: budget.max_sample_rate,
        }));
    }
    let channels_usize = usize::from(channels);
    if !samples.len().is_multiple_of(channels_usize) {
        return Err(DecodeError::InconsistentLayout {
            detail: format!(
                "{} interleaved samples is not a multiple of {channels} channels",
                samples.len()
            ),
        });
    }
    // 声道数闸门**在这里**判，也就是在任何分配之前、且与帧数无关。
    //
    // 改建前它只由下面的 `check_layout`（以及恒等分支里的那一次）判，而那两次都晚于
    // `Async::new_sinc`：重采样器在构造时就会分配 `channels` 份内部缓冲
    // （`rubato` 的 `vec![vec![0.0; chunk_size + 2 * sinc_len]; channels]`）。实测
    // （`/tmp` 计数分配器探针，65535 声道 = `u16::MAX`、1 帧、48 kHz → 96 kHz）：
    // 峰值存活**额外** 404 420 659 字节 ≈ 385.7 MiB，然后才返回 `TooManyChannels`；
    // 同一个输入走**恒等**路径（48 kHz → 48 kHz）时该读数是 **0 字节** —— 同一条闸门在
    // 同一个函数的两条分支上位置不同。
    //
    // 同一处还有第二个缺口：`frames == 0` 的提前返回位于 `check_layout` **之前**，因此
    // 「65535 声道 + 0 帧」改建前是 `Ok(空)`，而紧挨着的「0 声道 + 0 帧」是
    // `ZeroChannels` —— 同一道闸门的两半在 0 帧输入上得到相反待遇。判定与
    // `check_layout` 的首道闸门同源同值（[`LimitViolation::TooManyChannels`]），因此这里
    // **只提前判定，不改判据**；唯一可见的差异是「0 帧 + 超预算声道数」由放行改为拒绝，
    // 方向是收紧（判据 `an_over_budget_channel_count_is_refused_before_any_resampling`）。
    if channels > budget.max_channels {
        return Err(DecodeError::Budget(LimitViolation::TooManyChannels {
            channels,
            limit: budget.max_channels,
        }));
    }
    let frames = samples.len() / channels_usize;
    if frames == 0 {
        // 0 帧输入不消耗任何资源，因此**不**做比例判定 —— "非空输入才谈尺寸"是这条
        // 提前返回的既有语义（判据 `zero_frame_input_stays_zero_frame_output`），
        // 新闸门不在这里加宽它。
        return Ok(Vec::new());
    }
    // 比例闸门（第六道，判据 `an_over_the_cap_sample_rate_ratio_is_refused_before_any_resampling`）。
    //
    // 为什么这道闸门不可省：下面 `Async::new_sinc` 之后的那一步会按
    // `(CHUNK_FRAMES + SINC_LEN/2) × 比例` 要求输出缓冲 —— 与输入帧数**无关**。合法上界
    // （1 Hz → 768 kHz）下，**1 帧**输入要到 885 504 010 帧 ≈ 3.54 GB，而 `check_layout`
    // 的四道闸门全部放行（时长闸门按输出率折算只有 1 153 秒）。比例因此是资源放大的
    // 唯一来源，必须在**任何分配之前**单独判一次。
    //
    // 位置：它排在 `resample_interleaved_with_budget` 既有的**全部**前置判定与零帧提前
    // 返回之后，也就是最后一道前置闸门。这样任何在改建前被拒的输入报的还是原来那个错误
    // （零率、采样率越界、布局不整、超预算声道数都不被它改写），它只**新增**拒绝 ——
    // 唯一的例外是"0 帧 + 超比例对"，那一格仍然放行（见上面的提前返回）。
    limits::check_resample_ratio(in_rate, out_rate, budget)?;
    let frames_u64 = u64::try_from(frames).unwrap_or(u64::MAX);

    // 恒等路径：不做滤波，逐样本原样返回（位模式完全相同）—— 但它**仍然复制一整份
    // PCM**（`samples.to_vec()`），所以同样要过预算。
    if in_rate == out_rate {
        limits::check_resampled_len(frames_u64, out_rate, in_rate, frames_u64)?;
        limits::check_layout(channels, out_rate, frames_u64, budget)?;
        return Ok(samples.to_vec());
    }

    let ratio = f64::from(out_rate) / f64::from(in_rate);
    let parameters = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2);
    let mut resampler = Async::<f32>::new_sinc(
        ratio,
        MAX_RELATIVE_RATIO,
        &parameters,
        CHUNK_FRAMES,
        channels_usize,
        FixedAsync::Input,
    )
    .map_err(|err| DecodeError::ResamplerConfiguration {
        detail: format!("{err:?}"),
    })?;

    let needed_frames = resampler.process_all_needed_output_len(frames);
    let needed_samples = needed_frames
        .checked_mul(channels_usize)
        .ok_or(DecodeError::Budget(LimitViolation::LayoutOverflow {
            frames: u64::try_from(needed_frames).unwrap_or(u64::MAX),
            channels,
        }))?;
    limits::check_layout(
        channels,
        out_rate,
        u64::try_from(needed_frames).unwrap_or(u64::MAX),
        budget,
    )?;

    // `try_reserve` 而不是 `vec![0.0; n]`：分配失败要变成错误，不能 abort。
    let mut output: Vec<f32> = Vec::new();
    output.try_reserve(needed_samples).map_err(|_| {
        DecodeError::Budget(LimitViolation::AllocationRefused {
            samples: u64::try_from(needed_samples).unwrap_or(u64::MAX),
        })
    })?;
    output.resize(needed_samples, 0.0);

    let produced = {
        let input = InterleavedSlice::new(samples, channels_usize, frames).map_err(|err| {
            DecodeError::Resampling {
                detail: format!("input adapter: {err:?}"),
            }
        })?;
        let mut out = InterleavedSlice::new_mut(&mut output, channels_usize, needed_frames)
            .map_err(|err| DecodeError::Resampling {
                detail: format!("output adapter: {err:?}"),
            })?;
        let (_consumed, produced) = resampler
            .process_all_into_buffer(&input, &mut out, frames, None)
            .map_err(|err| DecodeError::Resampling {
                detail: format!("{err:?}"),
            })?;
        produced
    };

    let produced_samples = produced
        .checked_mul(channels_usize)
        .ok_or(DecodeError::Budget(LimitViolation::LayoutOverflow {
            frames: u64::try_from(produced).unwrap_or(u64::MAX),
            channels,
        }))?;
    if produced_samples > output.len() {
        return Err(DecodeError::InconsistentLayout {
            detail: format!(
                "resampler claims {produced} output frames but the buffer only holds {}",
                output.len() / channels_usize
            ),
        });
    }
    output.truncate(produced_samples);

    // 长度契约 [ARCH-DSP-002]：输入长度 → 输出长度必须是可预测的关系。
    limits::check_resampled_len(
        frames_u64,
        out_rate,
        in_rate,
        u64::try_from(produced).unwrap_or(u64::MAX),
    )?;
    limits::check_layout(
        channels,
        out_rate,
        u64::try_from(produced).unwrap_or(u64::MAX),
        budget,
    )?;

    Ok(output)
}

/// 把一个解码出的资产整体转换到 `out_rate`。
///
/// 语义：
/// - `out_rate == asset.sample_rate()` ⇒ 原样克隆（含 `pcm_hash`，因此"同率转换"可被
///   摘要判据直接证明是没有副作用的）；
/// - 否则返回一个**新的**资产：`sample_rate` 变为 `out_rate`，`pcm_format` 变为
///   [`PcmFormat::F32`]（重采样在 `f32` 域进行），`declared_bit_depth` /
///   `declared_frames` / 编码器延迟填充全部清空（它们描述的是**原容器**，对新资产不再成立，
///   留着就是撒谎）；
/// - 新资产的对账结论是 [`Reconciliation::DeclaredUnknown`]：重采样后的长度由本函数给出，
///   没有"容器声明"可对。
///
/// # Errors
///
/// 见 [`resample_interleaved`]。
pub fn resample_asset(asset: &DecodedAsset, out_rate: u32) -> DecodeResult<DecodedAsset> {
    resample_asset_with_budget(asset, out_rate, &PcmBudget::default())
}

/// 把一个解码出的资产整体转换到 `out_rate`，资源上限走 `budget`。
///
/// 语义与 [`resample_asset`] 完全一致，只是预算可配置：上采样会把长度放大
/// （48k → 192k 是 4×），因此转换后的新资产同样必须过 `budget` 的两道长度闸门。
///
/// # Errors
///
/// 见 [`resample_interleaved_with_budget`]。
pub fn resample_asset_with_budget(
    asset: &DecodedAsset,
    out_rate: u32,
    budget: &PcmBudget,
) -> DecodeResult<DecodedAsset> {
    if out_rate == asset.sample_rate() {
        // 恒等路径**也**过调用方的预算：`asset.clone()` 深拷贝一整份 PCM（`DecodedAsset`
        // 持有 `Vec<f32>`），与 `resample_interleaved_with_budget` 的恒等路径是同一件事。
        // 不查预算就等于留下一个"绕过 `budget` 复制整份资产"的入口（[ARCH-SEC-003]）。
        // 这里**不**另查长度契约：恒等转换的输出长度就是输入长度，`produced == input`
        // 是恒真式，查它不携带任何信息。
        limits::check_layout(
            asset.channels(),
            asset.sample_rate(),
            asset.frame_count(),
            budget,
        )?;
        // 第六道闸门（重采样比例）。`2026-10-10` 由裁决 `R31` 加上：改建前这一条只在
        // `resample_interleaved_with_budget` 里判，而它排在**那个函数的**恒等分支之前
        // ⇒ 同一份输入、同一份预算下，样本级入口报 `ResampleRatioTooHigh`、资产级入口
        // 返回 `Ok(asset.clone())`，两个**公开**入口对同一件事给出相反结论。
        //
        // 位置：排在 `check_layout` **之后**，与样本级入口的判定顺序对齐（那里也是
        // "声道数/采样率/时长/字节"先判、比例后判），因此多道闸门同时收紧时两个入口
        // 报的也是同一个错误。判据
        // `the_two_identity_paths_agree_on_all_six_gates` 双向钉住这一致性。
        limits::check_resample_ratio(asset.sample_rate(), out_rate, budget)?;
        return Ok(asset.clone());
    }
    let samples = resample_interleaved_with_budget(
        asset.samples(),
        asset.channels(),
        asset.sample_rate(),
        out_rate,
        budget,
    )?;
    let frames = u64::try_from(samples.len() / usize::from(asset.channels())).unwrap_or(u64::MAX);
    let converted = DecodedAsset::new(
        DecodeFacts {
            channels: asset.channels(),
            sample_rate: out_rate,
            pcm_format: PcmFormat::F32,
            declared_bit_depth: None,
            declared_frames: None,
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::DeclaredUnknown,
        },
        samples,
    );
    // 新资产同样受预算约束（重采样可能把长度放大，例如 48k -> 192k）。
    limits::check_layout(
        converted.channels(),
        converted.sample_rate(),
        frames,
        budget,
    )?;
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{DecodeOptions, decode_bytes};
    use crate::testfix::{WavFormat, WavSpec, encode_f32_samples, wav};

    /// 一段直流信号：所有声道都是 `level`。直流最容易被读成"有没有前置静音"。
    fn dc(frames: usize, channels: u16, level: f32) -> Vec<f32> {
        vec![level; frames * usize::from(channels)]
    }

    fn wav_f32(frames: usize, channels: u16, rate: u32, level: f32) -> Vec<u8> {
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits: 32,
            format: WavFormat::Float,
        };
        wav(&spec, &encode_f32_samples(&dc(frames, channels, level)))
    }

    #[test]
    fn configuration_is_pinned() {
        // 判据 (确定性配置): 这些数字是"同输入 → 同输出"的一部分，改它们必须同时改 notes。
        assert_eq!(SINC_LEN, 256);
        assert_eq!(CHUNK_FRAMES, 1024);
        assert_eq!(MAX_RELATIVE_RATIO, 1.0);
        assert_eq!(PREFILL_FRAMES, 0);
    }

    #[test]
    fn identity_rate_is_a_bit_exact_passthrough() {
        let input: Vec<f32> = (0..64).map(|i| (i as f32) * 0.01 - 0.3).collect();
        let output = resample_interleaved(&input, 2, 48_000, 48_000).unwrap();
        assert_eq!(
            output.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            input.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn resampling_48k_to_44k1_meets_the_length_contract() {
        let input = dc(48_000, 1, 1.0);
        let output = resample_interleaved(&input, 1, 48_000, 44_100).unwrap();
        let produced = u64::try_from(output.len()).unwrap();
        let contract = limits::check_resampled_len(48_000, 44_100, 48_000, produced)
            .expect("44100 frames of output for 1 s at 44.1 kHz");
        assert_eq!(contract.ideal_floor, 44_100);
    }

    #[test]
    fn resampling_48k_to_96k_doubles_the_length() {
        let input = dc(24_000, 1, 1.0);
        let output = resample_interleaved(&input, 1, 48_000, 96_000).unwrap();
        limits::check_resampled_len(24_000, 96_000, 48_000, u64::try_from(output.len()).unwrap())
            .expect("doubling the rate must double the frames");
    }

    #[test]
    fn resampling_44k1_to_48k_meets_the_length_contract() {
        let input = dc(44_100, 1, 1.0);
        let output = resample_interleaved(&input, 1, 44_100, 48_000).unwrap();
        limits::check_resampled_len(44_100, 48_000, 44_100, u64::try_from(output.len()).unwrap())
            .expect("48000 frames of output for 1 s at 48 kHz");
    }

    #[test]
    fn no_leading_pad_is_observable_as_an_immediate_half_level_crossing() {
        // 判据 (延迟/预填充语义): 输入是从第一帧就为 1.0 的直流阶跃。
        //
        // 阶跃响应在**被正确对齐**时，第 0 个输出样本就已经在 0.5（对称滤波器的阶跃中点）；
        // 而如果启动延迟没有被裁掉，0.5 交叉点会被推到 `output_delay()` 帧之后
        // ——sinc_len=256 时那是滤波器长度量级（远大于本判据允许的 4 帧）。
        // 用"交叉点位置"而不是"第 0 个样本等于 1.0"：后者对任何有限长滤波器都不可能成立，
        // 会变成一条永远红或永远被放宽的假判据。
        let input = dc(24_000, 2, 1.0);
        let output = resample_interleaved(&input, 2, 48_000, 44_100).unwrap();
        assert!(output.len() > 64);
        let crossing = output
            .iter()
            .position(|sample| *sample >= 0.5)
            .expect("output never reaches half of the input level");
        assert!(
            crossing <= 4,
            "the half-level crossing sits at output frame {crossing}; \
             the resampler's startup delay was not trimmed off"
        );
        // 中段必须严格保持在直流上（窗口完全落在输入内部的区间）。
        let frames = output.len() / 2;
        let lo = frames / 10;
        let hi = frames - frames / 10;
        for frame in lo..hi {
            let left = output[frame * 2];
            let right = output[frame * 2 + 1];
            assert!(
                (left - 1.0).abs() < 1e-2,
                "output frame {frame} left is {left}, not the input DC level"
            );
            assert!(
                (right - 1.0).abs() < 1e-2,
                "output frame {frame} right is {right}, not the input DC level"
            );
        }
    }

    #[test]
    fn channel_order_survives_resampling() {
        // 左声道 +1.0，右声道 -0.5：重采样后仍须分居两个声道。
        let frames = 8_000;
        let mut input = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            input.push(1.0f32);
            input.push(-0.5f32);
        }
        let output = resample_interleaved(&input, 2, 48_000, 96_000).unwrap();
        assert!(output.len().is_multiple_of(2));
        let frames = output.len() / 2;
        let lo = frames / 10;
        let hi = frames - frames / 10;
        for frame in lo..hi {
            let (left, right) = (output[frame * 2], output[frame * 2 + 1]);
            assert!(
                (left - 1.0).abs() < 1e-2,
                "left channel at frame {frame}: {left}"
            );
            assert!(
                (right + 0.5).abs() < 1e-2,
                "right channel at frame {frame}: {right}"
            );
        }
    }

    #[test]
    fn same_input_resamples_identically_in_two_threads() {
        // [ARCH-DET-001] 判据：固定算法 + 无随机 + 无全局状态 => 跨线程逐位相同。
        let input = dc(12_000, 1, 0.7);
        let first = resample_interleaved(&input, 1, 48_000, 44_100).unwrap();
        let second = std::thread::spawn(move || {
            let input = dc(12_000, 1, 0.7);
            resample_interleaved(&input, 1, 48_000, 44_100).unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(first.len(), second.len());
        assert_eq!(
            first.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            second.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn zero_frame_input_stays_zero_frame_output() {
        assert!(
            resample_interleaved(&[], 2, 48_000, 96_000)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn degenerate_arguments_are_rejected_rather_than_guessed() {
        assert!(matches!(
            resample_interleaved(&[0.0; 4], 0, 48_000, 96_000),
            Err(DecodeError::Budget(LimitViolation::ZeroChannels))
        ));
        assert!(matches!(
            resample_interleaved(&[0.0; 4], 1, 0, 96_000),
            Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
        ));
        assert!(matches!(
            resample_interleaved(&[0.0; 4], 1, 48_000, 0),
            Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
        ));
        assert!(matches!(
            resample_interleaved(&[0.0; 4], 1, 48_000, 2_000_000),
            Err(DecodeError::Budget(
                LimitViolation::SampleRateTooHigh { .. }
            ))
        ));
        // 交织长度不是声道数的整数倍。
        assert!(matches!(
            resample_interleaved(&[0.0; 5], 2, 48_000, 96_000),
            Err(DecodeError::InconsistentLayout { .. })
        ));
    }

    /// 判据（类别①：非有限输入）：非有限样本**传播**而不是被清洗，并且传播是确定性的、
    /// 不改变长度契约。
    ///
    /// 语义与"为什么这里不拒绝"的取舍写在 `resample_interleaved_with_budget` 的文档里
    /// （不可信字节的边界在 `decode`，那里会拒绝）。本判据只钉三件可观察的事：
    /// 1. **恒等路径逐位原样**：`in_rate == out_rate` 时输出与输入逐位相同（含 NaN 载荷与
    ///    ±∞ 的符号位）—— 同时证明恒等路径没有偷偷清洗；
    /// 2. **滤波路径两次调用逐位一致**（[ARCH-DET-001]）：同一份非有限输入不能因为传播
    ///    顺序而给出不同的位模式，且传播确实发生了（不是被清洗成 0）；
    /// 3. **长度契约与有限性无关**：同一形状的有限输入与非有限输入输出长度相同 ——
    ///    "样本是不是有限"与长度闸门是两件互不干扰的事。
    ///
    /// 注入：把恒等分支的 `samples.to_vec()` 换成
    /// `samples.iter().map(|s| if s.is_finite() { *s } else { 0.0 }).collect()` ⇒ 第 1 条红
    /// （NaN 的位型变成 `0.0` 的位型）；只在滤波分支上按 `is_finite` 过滤 ⇒ 第 3 条红。
    #[test]
    fn non_finite_input_propagates_deterministically_and_keeps_the_length_contract() {
        for (name, bad) in [
            ("NaN", f32::NAN),
            ("+inf", f32::INFINITY),
            ("-inf", f32::NEG_INFINITY),
        ] {
            let input = [bad, 1.0, 0.5, -1.0];

            let same = resample_interleaved(&input, 2, 48_000, 48_000).expect("identity path");
            assert_eq!(
                same.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                input.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                "{name}: the identity path must not sanitize"
            );

            let a = resample_interleaved(&input, 2, 48_000, 96_000).expect("filter path");
            let b = resample_interleaved(&input, 2, 48_000, 96_000).expect("filter path");
            assert_eq!(
                a.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                "{name}: non-finite propagation must be bit-deterministic"
            );
            assert!(
                a.iter().any(|s| !s.is_finite()),
                "{name}: the filter path must propagate, not sanitize"
            );

            let finite = [0.25, 1.0, 0.5, -1.0];
            let c = resample_interleaved(&finite, 2, 48_000, 96_000).expect("finite control");
            assert_eq!(
                a.len(),
                c.len(),
                "{name}: output length must not depend on finiteness"
            );
        }
    }

    /// 判据 (闸门站点 / 参数极值 + 块长度极值)：**声道数**闸门必须在任何分配之前、
    /// 且与帧数无关。
    ///
    /// 改建前 `channels > budget.max_channels` 只由 `check_layout` 判，而那两次调用都在
    /// `Async::new_sinc` **之后**（重采样器构造时会分配 `channels` 份内部缓冲），而且
    /// 非恒等路径的那一次还在 `frames == 0` 的提前返回之后。后果有两半，本判据把两半都钉住：
    ///
    /// - **资源**：65535 声道（`u16::MAX`）、1 帧、48 kHz → 96 kHz 改建前要先把重采样器
    ///   的内部缓冲分配出来再报 `TooManyChannels`。`/tmp` 的计数分配器探针读数是
    ///   **404 420 659 字节**（≈385.7 MiB）的峰值额外存活内存（= `65535 × (1024 + 2×256) × 4`
    ///   字节 + 约 1.77 MiB 的 sinc 表，与 `rubato` 的 `vec![vec![0.0; buffer_len]; channels]`
    ///   逐项对上）；同一输入走**恒等**路径（48 kHz → 48 kHz）时该读数是 **0 字节**。
    ///   ⚠ 这一半**不能**由本条判据自己证伪：两条分支改建前后都返回同一个
    ///   `TooManyChannels`，变的只是"判在分配之前还是之后"。因此第 ③ 组钉的是**形状**
    ///   （两条分支同一结论），资源那一半的证据是上面那支探针的前后读数
    ///   （404 420 659 B → 0 B）。本 crate 不能内置计数分配器：那要 `unsafe`，而
    ///   `lib.rs` 是 `#![forbid(unsafe_code)]`。
    /// - **判据形状（可注入的那一半）**：`frames == 0` 的提前返回改建前位于闸门之前，于是
    ///   「65535 声道 + 0 帧」是 `Ok(空)`，而紧挨着的「0 声道 + 0 帧」是 `ZeroChannels`。
    ///   同一道闸门的两半在 0 帧输入上得到相反的待遇。改建后两半都在提前返回之前判，
    ///   方向是**收紧**。
    ///
    /// 注入：删掉 `resample_interleaved_with_budget` 里新加的那次 `channels > max_channels`
    /// 判定 ⇒ 本条以 `expected TooManyChannels for 65 channels at 0 frames, got Ok([])` 红
    /// （第 ① 组；`cargo test -p yeban-decode --lib` 的读数是 **105 passed / 1 failed**，
    /// 即该注入只打红这一条 —— 全库没有别的判据钉这个位置）。
    #[test]
    fn an_over_budget_channel_count_is_refused_before_any_resampling() {
        // ① 0 帧 + 超预算声道数：不得走 "0 帧 ⇒ Ok(空)" 那个提前返回。
        //    比对的字面值与 `LimitViolation` 的字段名一起写出来，避免 `matches!` 把
        //    `limit` 写错也看不出来。
        match resample_interleaved(&[], 65, 48_000, 96_000) {
            Err(DecodeError::Budget(LimitViolation::TooManyChannels { channels, limit })) => {
                assert_eq!((channels, limit), (65, 64));
            }
            other => panic!("expected TooManyChannels for 65 channels at 0 frames, got {other:?}"),
        }

        // ② 对照：0 帧 + 预算内声道数仍然是 `Ok(空)` —— 这条闸门不是"拒绝一切空输入"，
        //    0 帧的既有语义（见 `zero_frame_input_stays_zero_frame_output`）原样保留。
        assert!(
            resample_interleaved(&[], 2, 48_000, 96_000)
                .expect("0 frames with 2 channels still yields an empty output")
                .is_empty()
        );

        // ③ `u16::MAX` 声道、1 帧：**恒等**与**非恒等**两条分支给出同一条判据（改建前
        //    非恒等那一条要先把 385.7 MiB 的内部缓冲分配出来才报同一个错）。
        let widest = vec![0.0f32; 65_535];
        for (in_rate, out_rate) in [(48_000u32, 96_000u32), (48_000, 48_000)] {
            let outcome = resample_interleaved(&widest, 65_535, in_rate, out_rate);
            assert!(
                matches!(
                    outcome,
                    Err(DecodeError::Budget(LimitViolation::TooManyChannels {
                        channels: 65_535,
                        limit: 64
                    }))
                ),
                "{in_rate} -> {out_rate}: got {outcome:?}"
            );
        }

        // ④ 闭区间：恰好 `max_channels` 个声道必须通过（闸门不是 `>=`）。64 声道 × 2 帧
        //    的输出很短，因此这条断言不会把默认预算撑破。
        let at_cap = resample_interleaved(&[0.0; 128], 64, 48_000, 96_000)
            .expect("exactly max_channels must pass the channel gate");
        assert_eq!(at_cap.len(), 256);

        // ⑤ 闸门真的读调用方的预算，而不是写死的 64：`max_channels = 1` 时立体声即拒。
        let mono_only = PcmBudget::new(
            u64::MAX,
            1 << 30,
            1,
            768_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        match resample_interleaved_with_budget(&[0.0; 4], 2, 48_000, 96_000, &mono_only) {
            Err(DecodeError::Budget(LimitViolation::TooManyChannels { channels, limit })) => {
                assert_eq!((channels, limit), (2, 1));
            }
            other => panic!("expected TooManyChannels at a 1-channel cap, got {other:?}"),
        }

        // ⑥ 相邻优先级没有被改动：`samples.len()` 不是整数倍时仍然是布局错误，
        //    采样率为 0 时仍然是采样率错误（两者都排在声道数闸门之前，与
        //    `resample_interleaved_with_budget` 的判定顺序一致）。
        assert!(matches!(
            resample_interleaved(&[0.0; 5], 65, 48_000, 96_000),
            Err(DecodeError::InconsistentLayout { .. })
        ));
        assert!(matches!(
            resample_interleaved(&[], 65, 0, 96_000),
            Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
        ));
    }

    #[test]
    fn resample_asset_updates_the_facts_and_the_hash() {
        let bytes = wav_f32(12_000, 2, 48_000, 0.5);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.sample_rate(), 48_000);

        let converted = resample_asset(&asset, 44_100).unwrap();
        assert_eq!(converted.sample_rate(), 44_100);
        assert_eq!(converted.channels(), 2);
        assert_eq!(converted.pcm_format(), PcmFormat::F32);
        // 原容器的声明不跟着搬过来。
        assert_eq!(converted.declared_frames(), None);
        assert_eq!(converted.facts().declared_bit_depth, None);
        assert_eq!(converted.facts().encoder_delay_frames, None);
        assert_eq!(converted.facts().duration, Reconciliation::DeclaredUnknown);
        assert_ne!(converted.pcm_hash(), asset.pcm_hash());

        // 同率转换必须是**无副作用**的：事实、样本、摘要全都一样。
        let same = resample_asset(&asset, 48_000).unwrap();
        assert_eq!(same.pcm_hash(), asset.pcm_hash());
        assert_eq!(same.facts(), asset.facts());
        assert_eq!(same.samples(), asset.samples());
    }

    /// 判据 ⑧（上游口径）：裸入口就是"默认预算入口"，两者结果逐位相同。
    #[test]
    fn the_plain_entry_points_are_the_default_budget_entry_points() {
        let input = dc(1_024, 2, 0.25);
        assert_eq!(
            resample_interleaved(&input, 2, 48_000, 96_000).unwrap(),
            resample_interleaved_with_budget(&input, 2, 48_000, 96_000, &PcmBudget::default())
                .unwrap()
        );
        let bytes = wav_f32(2_048, 2, 48_000, 0.25);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(
            resample_asset(&asset, 96_000).unwrap().samples(),
            resample_asset_with_budget(&asset, 96_000, &PcmBudget::default())
                .unwrap()
                .samples()
        );
    }

    /// 判据 ④/⑧：重采样**真的**走调用方的预算，而不是写死的常量。
    #[test]
    fn the_resampler_budget_is_the_callers_budget_not_a_hard_coded_cap() {
        let input = dc(48_000, 1, 1.0); // 1 秒 @48k 单声道
        assert!(resample_interleaved(&input, 1, 48_000, 44_100).is_ok());

        // 只够"理想输出"的预算会被拒：闸门判定的是**实际要分配的**输出缓冲
        // （`process_all_needed_output_len`，含滤波器延迟/余量），不是理想长度。
        let ideal_only = PcmBudget::new(
            u64::MAX,
            44_100 * 4,
            64,
            768_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        let err =
            resample_interleaved_with_budget(&input, 1, 48_000, 44_100, &ideal_only).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::PcmBudgetExceeded { .. })
            ),
            "expected a budget refusal, got {err}"
        );
        assert!(err.to_string().contains("PCM budget"), "got {err}");

        // 给足余量后同一份输入必须成功 ⇒ 上面红的是预算，不是参数写错。
        let roomy = PcmBudget::new(
            u64::MAX,
            8 * 1024 * 1024,
            64,
            768_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        let out = resample_interleaved_with_budget(&input, 1, 48_000, 44_100, &roomy).unwrap();
        limits::check_resampled_len(48_000, 44_100, 48_000, u64::try_from(out.len()).unwrap())
            .expect("length contract");

        // 采样率闸门也走调用方的预算（默认预算是 768 kHz）。
        assert!(matches!(
            resample_interleaved_with_budget(
                &input,
                1,
                48_000,
                44_100,
                &PcmBudget::new(
                    u64::MAX,
                    1 << 30,
                    64,
                    44_100,
                    60,
                    limits::DEFAULT_MAX_RESAMPLE_RATIO
                )
            ),
            Err(DecodeError::Budget(
                LimitViolation::SampleRateTooHigh { .. }
            ))
        ));
    }

    /// 判据 ⑤/⑧：重采样输出的**时长闸门**与字节预算各自独立。
    #[test]
    fn the_resampled_length_gate_is_independent_of_the_byte_budget() {
        // 2 秒 @48k 单声道 ⇒ 96k 输出 2 秒（约 192000 帧）。字节预算宽到用不完。
        let input = dc(96_000, 1, 0.5);
        let by_time = PcmBudget::new(
            u64::MAX,
            !3u64,
            64,
            192_000,
            1,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        let err =
            resample_interleaved_with_budget(&input, 1, 48_000, 96_000, &by_time).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::DurationTooLong { .. })
            ),
            "expected DurationTooLong, got {err}"
        );
        assert!(
            resample_interleaved_with_budget(&input, 1, 48_000, 96_000, &PcmBudget::default())
                .is_ok()
        );
    }

    /// 判据 ⑧：恒等路径（`in_rate == out_rate`）同样要过预算 —— 它仍会复制一整份 PCM。
    #[test]
    fn the_identity_path_still_obeys_the_budget() {
        let input = dc(48_000, 1, 0.25);
        let tiny = PcmBudget::new(
            u64::MAX,
            8,
            1,
            96_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        assert!(matches!(
            resample_interleaved_with_budget(&input, 1, 48_000, 48_000, &tiny),
            Err(DecodeError::Budget(
                LimitViolation::PcmBudgetExceeded { .. }
            ))
        ));
        assert_eq!(
            resample_interleaved_with_budget(&input, 1, 48_000, 48_000, &PcmBudget::default())
                .unwrap()
                .len(),
            input.len()
        );
    }

    /// 判据 ⑧（`HD-24` 预算闸门，[ARCH-SEC-003]）：**资产级**恒等路径同样走调用方预算。
    ///
    /// 为什么需要它：`resample_asset_with_budget` 的恒等分支走 `asset.clone()`，而
    /// [`DecodedAsset`] 持有 `Vec<f32>`（`#[derive(Clone)]`），因此它是一次**整份 PCM
    /// 的深拷贝**。既有判据 `the_identity_path_still_obeys_the_budget` 只覆盖样本级入口
    /// `resample_interleaved_with_budget`，于是资产级入口成了"绕过 `budget` 复制整份
    /// 资产"的口子 —— 与 `docs/ledger/decode-limits-notes.md` §3 写明的改建后口径
    /// "恒等路径也查（它同样复制一整份 PCM）"不符。
    ///
    /// 判定是闭区间：恰好装得下资产的预算**必须**通过（否则就是把闸门焊死），而少一个
    /// 样本即拒；通过时必须逐位不变 —— 预算只挡分配，不改内容。
    #[test]
    fn the_asset_identity_path_still_obeys_the_budget() {
        let bytes = wav_f32(2_000, 2, 48_000, 0.5);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        let frames = asset.frame_count();
        let samples = frames * u64::from(asset.channels());
        assert_eq!(frames, 2_000);

        // 少一个样本的预算：恒等路径仍要复制整份 PCM，因此必须被拒。
        let one_sample_short = PcmBudget::new(
            u64::MAX,
            (samples - 1) * 4,
            64,
            96_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        assert!(
            matches!(
                resample_asset_with_budget(&asset, 48_000, &one_sample_short),
                Err(DecodeError::Budget(
                    LimitViolation::PcmBudgetExceeded { .. }
                ))
            ),
            "the asset-level identity path copies the whole PCM, so it must obey the budget"
        );

        // 恰好够的预算必须通过，且样本 / 事实 / 摘要逐位不变。
        let exact = PcmBudget::new(
            u64::MAX,
            samples * 4,
            64,
            96_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        let same = resample_asset_with_budget(&asset, 48_000, &exact).unwrap();
        assert_eq!(same.pcm_hash(), asset.pcm_hash());
        assert_eq!(same.facts(), asset.facts());
        assert_eq!(same.samples(), asset.samples());
    }

    /// 判据 ⑧：`resample_asset_with_budget` 把同一预算应用到"转换后的新资产"。
    #[test]
    fn resample_asset_with_budget_refuses_output_over_the_budget() {
        let bytes = wav_f32(12_000, 2, 48_000, 0.5);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        // 48k → 96k 输出翻倍：只够原始长度的预算必须被拒。
        let small = PcmBudget::new(
            u64::MAX,
            12_000 * 2 * 4,
            64,
            96_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        assert!(matches!(
            resample_asset_with_budget(&asset, 96_000, &small),
            Err(DecodeError::Budget(_))
        ));
        let converted = resample_asset_with_budget(&asset, 96_000, &PcmBudget::default()).unwrap();
        assert_eq!(converted.sample_rate(), 96_000);
        assert!(converted.frame_count() >= 24_000);
    }

    /// 样本的位模式。类别⑤与类别⑥的判据只承认**逐位**相同，不承认"值相等"
    /// （`-0.0` 与 `+0.0` 值相等而位模式不等，`pcm_hash` 也区分二者）。
    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|sample| sample.to_bits()).collect()
    }

    /// 判据（类别⑤ 幂等性）：**四个重采样入口**在同一个对象上重复施加同一个值，与只施加
    /// 一次逐位相同；把同一个转换再施加到**它自己的输出**上（输出资产的采样率已经等于
    /// `out_rate`）也得到同一个资产。
    ///
    /// 机械枚举的口径（全 crate，`grep -rnw` 加词边界）：非测试代码里属于"把同一个值施加
    /// 到同一个对象"这一形态的公共入口**只有** `resample_interleaved` /
    /// `resample_interleaved_with_budget` / `resample_asset` / `resample_asset_with_budget`
    /// 四个。全 crate 唯一的可变状态对象是 `limits::IdleGuard`（`&mut self` 的非测试
    /// 出现只有 `bump` 与 `reset` 两处），它的重复复位由 `limits` 侧的判据
    /// `repeated_resets_leave_the_guard_in_the_same_state_as_a_single_reset` 钉住。
    /// `decode_*` / `import_*` 的重复施加已由 `decoding_the_same_bytes_twice_is_bit_identical`
    /// 与 `importing_the_same_bytes_twice_yields_the_same_keys` 钉住；`asset_index` /
    /// `reconcile` / `limits::*` 是纯函数（无状态、无 I/O），重复调用即同一次调用。
    ///
    /// 非确定性来源的机械排查（全 crate、加词边界；逐项命令与读数见本判据的提交信息）：
    /// 时钟读取只有 **1 行**，而且它在 `decode.rs` 的测试模块内（临时文件名），
    /// 非测试代码里是 **0 处**；PRNG、UUID、单调时钟、哈希容器与环境变量读取逐项都是
    /// 0 处。因此重复施加的输出里**没有**需要登记为"允许"的时钟或 ULID：内容只有样本
    /// 位模式、`DecodeFacts` 与 SHA-256（后者本身是内容的函数）。
    ///
    /// 注入：在非恒等路径的 `truncate` 之后按**调用计数器**给第 0 个样本加 `1.0e-7`
    /// （偶数次调用加、奇数次不加）⇒ 本条以 `resample_interleaved repeated at
    /// 48000 -> 44100` 红，位模式首元素是 `3198797749` 对 `3198797746`（相差 3 个 ULP）；
    /// 同一批里 `same_input_resamples_identically_in_two_threads` 也红，读数是
    /// `108 passed / 3 failed`。
    #[test]
    fn every_resample_entry_is_idempotent_on_repeated_application() {
        // 1001 帧：非 2 的幂、也不是 `CHUNK_FRAMES` 的整数倍，因此分块余数与既有判据
        // （12000 / 24000 / 48000 帧）不同。
        let frames = 1_001usize;
        let input: Vec<f32> = (0..frames)
            .map(|index| (index % 61) as f32 * 0.01 - 0.3)
            .collect();
        for (in_rate, out_rate) in [
            (48_000u32, 44_100u32),
            (48_000, 96_000),
            (44_100, 48_000),
            (48_000, 48_000),
        ] {
            let first = resample_interleaved(&input, 1, in_rate, out_rate)
                .unwrap_or_else(|err| panic!("{in_rate} -> {out_rate}: {err}"));
            let second = resample_interleaved(&input, 1, in_rate, out_rate)
                .unwrap_or_else(|err| panic!("{in_rate} -> {out_rate}: {err}"));
            assert_eq!(
                bits(&first),
                bits(&second),
                "resample_interleaved repeated at {in_rate} -> {out_rate}"
            );

            let first_budgeted = resample_interleaved_with_budget(
                &input,
                1,
                in_rate,
                out_rate,
                &PcmBudget::default(),
            )
            .unwrap();
            let second_budgeted = resample_interleaved_with_budget(
                &input,
                1,
                in_rate,
                out_rate,
                &PcmBudget::default(),
            )
            .unwrap();
            assert_eq!(
                bits(&first_budgeted),
                bits(&second_budgeted),
                "resample_interleaved_with_budget repeated at {in_rate} -> {out_rate}"
            );
            // 裸入口就是"默认预算入口"（见 `the_plain_entry_points_are_the_default_budget_
            // entry_points`），因此两条入口的重复施加必须是同一个读数。
            assert_eq!(bits(&first), bits(&first_budgeted));
        }

        // 资产级入口：同一个资产重复转换，样本 / 事实 / 摘要三样都必须逐位相同。
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 32,
            format: WavFormat::Float,
        };
        let mut interleaved = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let value = (index % 61) as f32 * 0.01 - 0.3;
            interleaved.push(value);
            interleaved.push(-value);
        }
        let bytes = wav(&spec, &encode_f32_samples(&interleaved));
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        for out_rate in [44_100u32, 96_000, 48_000] {
            let first = resample_asset(&asset, out_rate).unwrap();
            let second = resample_asset(&asset, out_rate).unwrap();
            assert_eq!(first.facts(), second.facts(), "facts at {out_rate}");
            assert_eq!(
                first.pcm_hash(),
                second.pcm_hash(),
                "pcm_hash at {out_rate}"
            );
            assert_eq!(bits(first.samples()), bits(second.samples()));

            let first_budgeted =
                resample_asset_with_budget(&asset, out_rate, &PcmBudget::default()).unwrap();
            let second_budgeted =
                resample_asset_with_budget(&asset, out_rate, &PcmBudget::default()).unwrap();
            assert_eq!(first_budgeted.pcm_hash(), second_budgeted.pcm_hash());
            assert_eq!(
                bits(first_budgeted.samples()),
                bits(second_budgeted.samples())
            );
            assert_eq!(first.pcm_hash(), first_budgeted.pcm_hash());

            // 幂等投影：把同一个转换再施加到它自己的输出上。第二次的 `out_rate` 已经等于
            // 该资产的采样率，因此走的是恒等分支 —— 结果必须与只施加一次逐位相同。
            let again = resample_asset(&first, out_rate).unwrap();
            assert_eq!(
                again.pcm_hash(),
                first.pcm_hash(),
                "idempotent projection at {out_rate}"
            );
            assert_eq!(bits(again.samples()), bits(first.samples()));
            assert_eq!(again.facts(), first.facts());
        }
    }

    /// 判据（类别⑦ 块长度极值）：1 帧、2 帧、非 2 的幂帧长、以及 [`CHUNK_FRAMES`]
    /// （1024）边界两侧都必须按长度契约返回，并且**非空输入绝不产出空输出**。
    ///
    /// 为什么需要这条：长度契约对短片段会追加 [`limits::SINC_LEN`] 帧的额外放宽，于是这些
    /// 长度的契约下界是 **0**（实测：1 帧 48 kHz → 44.1 kHz 的契约是 `0..=265`）。
    /// "输出 0 帧"这种回归因此能穿过契约判据，与"正确裁掉了启动延迟"无法区分。本判据改用
    /// `produced >= 1` 直接钉住"非空输入 ⇒ 非空输出"，并把 2 倍比例下的**精确**输出帧数
    /// （= 2 × 输入帧数）钉成字面值。
    ///
    /// 实测读数（本机 aarch64、debug 构建）：48 kHz → 96 kHz 下本判据的每一种帧数都
    /// **恰好**翻倍（1→2、2→4、3→6、5→10、7→14、8→16、63→126、255→510、1023→2046、
    /// 1024→2048、1025→2050、2047→4094）；48 kHz → 44.1 kHz 下是 1→1、2→2、3→3、5→5、
    /// 7→7、8→8、63→58、255→235、1023→940、1024→941、1025→942、2047→1881。
    /// 比例 2.0 是精确的二进制数，输出帧数只由整数索引运算给出，因此"恰好翻倍"与 SIMD
    /// 后端无关（`rubato` 的 `nbr_points()` 在 AVX / SSE / NEON / 标量四套实现里都是
    /// `round_sinc_len(256) = 256`）。
    ///
    /// 0 帧输入**不**在本判据里重复：实现侧由 `zero_frame_input_stays_zero_frame_output`
    /// 钉住，契约侧由 `limits` 的 `a_zero_frame_input_admits_exactly_zero_output_frames`
    /// 钉住。
    ///
    /// 注入：把 0 帧的提前返回从 `frames == 0` 放宽成 `frames <= 1` ⇒ 本条以
    /// `1 frames at 48000 -> 44100 produced an empty output` 红，读数是
    /// `109 passed / 2 failed`（另一条红的是类别⑥的判据，因为它的帧数表里也有 1 帧）。
    #[test]
    fn tiny_and_non_power_of_two_block_lengths_stay_inside_the_contract() {
        let frame_counts: [usize; 12] = [1, 2, 3, 5, 7, 8, 63, 255, 1_023, 1_024, 1_025, 2_047];
        for frames in frame_counts {
            let input = dc(frames, 1, 0.5);
            for (in_rate, out_rate) in [
                (48_000u32, 44_100u32),
                (48_000, 96_000),
                (44_100, 48_000),
                (48_000, 48_000),
            ] {
                let output =
                    resample_interleaved(&input, 1, in_rate, out_rate).unwrap_or_else(|err| {
                        panic!("{frames} frames at {in_rate} -> {out_rate}: {err}")
                    });
                let produced = u64::try_from(output.len()).unwrap();
                // ① 非空输入 ⇒ 非空输出。契约对短片段的下界是 0，因此这条**不是**契约的推论。
                assert!(
                    produced >= 1,
                    "{frames} frames at {in_rate} -> {out_rate} produced an empty output"
                );
                // ② 输出必须落在契约里 —— 长度关系的唯一数值来源。
                let contract = limits::check_resampled_len(
                    u64::try_from(frames).unwrap(),
                    out_rate,
                    in_rate,
                    produced,
                )
                .unwrap_or_else(|err| panic!("{frames} frames at {in_rate} -> {out_rate}: {err}"));
                assert!(contract.min <= produced && produced <= contract.max);
                // ③ 恒等比例（`in_rate == out_rate`）必须逐位原样返回，1 帧也不例外。
                if in_rate == out_rate {
                    assert_eq!(bits(&output), bits(&input), "identity at {frames} frames");
                }
            }
            // ④ 2 倍比例下输出帧数精确翻倍。契约对 1 帧输入允许 `0..=266`，因此这一步是
            //    比契约更强的一条判据。
            let doubled = resample_interleaved(&input, 1, 48_000, 96_000).unwrap();
            assert_eq!(
                doubled.len(),
                2 * frames,
                "48 kHz -> 96 kHz must double {frames} frames exactly"
            );
        }
    }

    /// 判据（类别⑥ 多声道一致性）：同一信号喂两路 ⇒ 两路输出**逐位**相同；一路为 `0.0`
    /// 的立体声 ⇒ 该路输出**逐位为 `+0.0`**（包含上采样、下采样、1 帧与长块）。
    ///
    /// 为什么必须逐位而不是"值相等"：一个"只对某一路按另一种权重求和"的缺陷在近似比较下
    /// 会静默通过，而 `pcm_hash` 与 L2 对账都按位模式判。
    ///
    /// 实测读数（本机 aarch64、debug 构建）：5000 帧的同一信号在 48 kHz → 44.1 kHz、
    /// 48 kHz → 96 kHz、44.1 kHz → 48 kHz 三种转换下，`L != R` 的输出帧数都是 **0**
    /// （输出分别是 4594 / 10000 / 5443 帧）；一路恒为 `0.0` 时，三种转换 × 帧数
    /// {1, 2, 7, 100, 5000} 的 15 组里非 `+0.0` 的输出样本数是 **0**。
    ///
    /// 注入：在非恒等路径的 `truncate` 之后**确定性**地把 `output[1]` 置成 `1.0e-9` ⇒
    /// 本条以 `48000 -> 44100: both channels carry the same input` 红（`left: 1` /
    /// `right: 0`），而**只有**本条红（`110 passed / 1 failed`）—— 这条注入是确定性的，
    /// 因此类别⑤那条幂等判据照样绿：两条判据量的是不同的东西。
    #[test]
    fn identical_channels_resample_bit_identically_and_a_silent_channel_stays_silent() {
        // 非直流信号：只用整数运算生成，不调用任何超越函数。
        let frames = 5_000usize;
        let mut input = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let value = (index % 37) as f32 * 0.02 - 0.4;
            input.push(value);
            input.push(value);
        }
        for (in_rate, out_rate) in [(48_000u32, 44_100u32), (48_000, 96_000), (44_100, 48_000)] {
            let output = resample_interleaved(&input, 2, in_rate, out_rate).unwrap();
            assert!(output.len().is_multiple_of(2));
            let mismatched = output
                .chunks(2)
                .filter(|pair| pair[0].to_bits() != pair[1].to_bits())
                .count();
            assert_eq!(
                mismatched, 0,
                "{in_rate} -> {out_rate}: both channels carry the same input"
            );
        }

        // 单声道信号喂立体声器件：只填一路，另一路恒为 0.0。
        for (in_rate, out_rate) in [(48_000u32, 96_000u32), (48_000, 44_100), (44_100, 48_000)] {
            for frames in [1usize, 2, 7, 100, 5_000] {
                let mut stereo = Vec::with_capacity(frames * 2);
                for index in 0..frames {
                    stereo.push((index % 37) as f32 * 0.02 - 0.4);
                    stereo.push(0.0);
                }
                let output = resample_interleaved(&stereo, 2, in_rate, out_rate).unwrap();
                let silent = output
                    .chunks(2)
                    .filter(|pair| pair[1].to_bits() == 0)
                    .count();
                assert_eq!(
                    silent,
                    output.len() / 2,
                    "{frames} frames at {in_rate} -> {out_rate}: the silent channel must stay +0.0"
                );
                // 非空洞证据：被驱动的那一路**不**是零，否则"两路都寂静"也会让上一条变绿。
                assert!(
                    output.chunks(2).any(|pair| pair[0].to_bits() != 0),
                    "{frames} frames at {in_rate} -> {out_rate}: the driven channel must carry signal"
                );
            }
        }
    }

    /// 判据（⑥-7 类别⑥ 多声道一致性）：**单声道输入**与"把同一路信号复制进 N 个声道"的输入，
    /// 在重采样后必须给出（a）精确的长度关系、（b）同一次调用内部**逐位**相同的各声道、
    /// 以及（c）与单声道结果在**已声明容差**内一致的每一个样本。
    ///
    /// 为什么需要它：既有的 ⑥ 判据只钉"2 声道、两路相同 ⇒ 两路输出逐位相同"与"一路恒 0
    /// ⇒ 该路输出逐位 +0.0"。**单声道喂多声道**（`channels = 1` 与 `channels = N` 的
    /// 交叉一致性）此前是空白，而这正是"不丢声道、不复制错声道"的落点。
    ///
    /// 为什么（c）**不是**逐位判据（这是一条实测结论，不是放宽）：`rubato` 自己的判据
    /// （`asynchro.rs` 的 `process_one_block_*` 系列）对"1 声道与 4 声道跑同一路信号"用的
    /// 就是容差 —— 它断言 `diff < 1e-10`（`f64`），并在注释里写明"4ch 输出必须在浮点容差内
    /// 与 1ch 输出一致"。本 crate 工作在 `f32`，实测（本机 `/tmp` 探针，`-O` 构建、
    /// 逐样本 `abs(mono[i] - many[i * channels])` 的最大值）：48 kHz → 44.1 kHz 是
    /// 1.19e-7（1001 帧）/ 1.49e-7（5000 帧），48 kHz → 96 kHz 是 5.96e-8，
    /// 44.1 kHz → 48 kHz 是 1.49e-7；而**恒等**比例（48 kHz → 48 kHz）在被测的每一种
    /// 帧数下都逐位相同（差分 0）。因此"跨声道数逐位相同"不是 `rubato` 的契约，把它写成
    /// 逐位判据会是一条**永远红**的假判据。⇒ 本条把逐位的部分放在（b）（同一次调用内部），
    /// 把跨声道数的一致性放在（c）并给出实测读数与 6× 余量的界。
    ///
    /// 实测读数（本机，5_000 帧、`channels ∈ {2, 3, 4, 8}`、三组采样率）：
    /// （a）`many.len() == single.len() * channels` 全部成立；
    /// （b）同一次调用内部的声道间位模式失配数是 **0**；
    /// （c）跨声道数的最大绝对差逐组为 48 kHz→44.1 kHz **1.19e-7**、
    /// 48 kHz→96 kHz **5.96e-8**、44.1 kHz→48 kHz **1.49e-7**（界取 `1.0e-6`，约 6.7× 余量）。
    ///
    /// 注入（证明本条不是空判据）：在非恒等路径的 `output.truncate(produced_samples)`
    /// 之后加一句"若 `output.len() > 1` 则 `output[1] += 1.0e-3`"（把第 0 帧的第 1 个声道
    /// 推离它的同路伙伴）⇒ 本条在（b）处以
    /// `2 channels at 48000 -> 44100: channels carrying the same input must be bit-identical
    /// inside one call` 红；整库读数是 `112 passed / 2 failed`（另一条红的是既有的 ⑥-3
    /// `identical_channels_resample_bit_identically_and_a_silent_channel_stays_silent`）。
    #[test]
    fn a_mono_input_and_an_n_channel_copy_agree_per_channel() {
        // 非直流信号：只用整数运算生成，不调用任何超越函数。帧数取 5_000 —— 与
        // `/tmp` 探针量出（c）的读数时用的夹具逐参数相同，因此文档里的字面值可直接复算。
        let frames = 5_000usize;
        let mono: Vec<f32> = (0..frames)
            .map(|index| (index % 37) as f32 * 0.02 - 0.4)
            .collect();
        for channels in [2u16, 3, 4, 8] {
            let mut interleaved = Vec::with_capacity(frames * usize::from(channels));
            for sample in &mono {
                for _ in 0..channels {
                    interleaved.push(*sample);
                }
            }
            for (in_rate, out_rate) in [
                (48_000u32, 44_100u32),
                (48_000, 96_000),
                (44_100, 48_000),
                (48_000, 48_000),
            ] {
                let single = resample_interleaved(&mono, 1, in_rate, out_rate)
                    .unwrap_or_else(|err| panic!("mono {in_rate} -> {out_rate}: {err}"));
                let many = resample_interleaved(&interleaved, channels, in_rate, out_rate)
                    .unwrap_or_else(|err| {
                        panic!("{channels} channels {in_rate} -> {out_rate}: {err}")
                    });

                // （a）长度关系是**精确**的：每个声道的帧数与单声道逐帧对齐，不多不少。
                assert_eq!(
                    many.len(),
                    single.len() * usize::from(channels),
                    "{channels} channels at {in_rate} -> {out_rate}: \
                     the output must hold the same frame count in every channel"
                );

                // （b）同一次调用内部，被复制出来的各声道**逐位**相同（上游契约的一半）。
                let width = usize::from(channels);
                let mismatched = many
                    .chunks(width)
                    .flat_map(|frame| frame[1..].iter().map(move |sample| (frame[0], *sample)))
                    .filter(|(first, other)| first.to_bits() != other.to_bits())
                    .count();
                assert_eq!(
                    mismatched, 0,
                    "{channels} channels at {in_rate} -> {out_rate}: \
                     channels carrying the same input must be bit-identical inside one call"
                );

                // （c）跨声道数的一致性：在实测读数（≤1.49e-7）之上留 6× 余量，用来抓
                //     "某个声道被换掉/清零/写进别的声道的数据"，而不是重新钉上游的舍入。
                let mut worst = 0.0f32;
                for (index, sample) in single.iter().enumerate() {
                    let diff = (many[index * width] - sample).abs();
                    if diff > worst {
                        worst = diff;
                    }
                }
                assert!(
                    worst <= 1.0e-6,
                    "{channels} channels at {in_rate} -> {out_rate}: \
                     the first channel differs from the mono result by {worst:e}"
                );

                // （d）非空洞对照：这一路信号不是常数，否则上面三条对"全零输出"也成立。
                assert!(
                    single.iter().any(|sample| sample.abs() > 1.0e-3),
                    "{in_rate} -> {out_rate}: the fixture must carry signal"
                );

                // （e）恒等比例是逐样本原样返回，因此那一条路径上跨声道数也必须**逐位**相同
                //     —— 与（c）的容差无关，是（c）不能变成"永远绿"的对照。
                if in_rate == out_rate {
                    assert!(
                        worst == 0.0,
                        "{channels} channels at {in_rate} -> {out_rate}: \
                         the identity path must be a bit-exact copy"
                    );
                }
            }
        }
    }

    /// 判据（类别④ 参数极值／类别⑦ 块长度极值）：重采样器在**短于一个分块**的输入上
    /// 要求的工作量是 `(CHUNK_FRAMES + SINC_LEN/2) × 比例`，**与输入帧数无关**。
    ///
    /// 量什么：`resample_interleaved_with_budget` 在任何分配之前交给
    /// [`limits::check_layout`] 的那个 `frames` 数（单位：帧），也就是输出缓冲要装下的
    /// 帧数。怎么量：把它交给一个**时长上限紧到必然跳闸**的预算，从
    /// [`LimitViolation::DurationTooLong`] 的 `frames` 字段里**读**出来 —— 判定发生在
    /// 分配与 `Async::new_sinc` 之后、`process_all_into_buffer` 之前，因此这条判据不会
    /// 真的跑重采样（不烧 CPU，也不受机器速度影响）。
    ///
    /// 读数（ratio = 输出率 / 输入率）：
    ///
    /// | 输入帧 | 输入率 → 输出率 | 比例 | 理想输出帧数 | 实际要求帧数 | 放大 |
    /// | :--- | :--- | :--- | :--- | :--- | :--- |
    /// | 1 | 2 Hz → 8 kHz | 4 000 | 4 000 | 4 612 010 | 1 153.0× |
    /// | 1 | 2 Hz → 8 kHz（4 声道） | 4 000 | — | 4 612 010 | 同上（与声道数无关） |
    /// | 1 | 1 Hz → 768 kHz | 768 000 | 768 000 | 885 504 010 | 1 153.0× |
    ///
    /// `4 612 010 = 1024×4 000 + 10 + 128×4 000 + 4 000`（分块项 + rubato 的常量 10 +
    /// 前置延迟项 + 理想输出）。**默认预算放行最后一行**：`885 504 010` 帧 × 4 字节
    /// ≈ 3.30 GiB 的 `f32`，而它是**1 帧输入**；`check_layout` 的四道闸门没有一道会挡它
    /// （时长闸门按 `frames / 输出率` 算，而输出率被极端放大；字节闸门是 96 kHz 立体声
    /// 3 小时的 8.29 GB）。本判据把这两个数字钉住，好让下一位不必重新测。
    ///
    /// 时间代价（`/tmp` 探针，本机 debug 构建，未在 CI 上测过；release 计时**未测** ——
    /// 本机不许编译 symphonia 重依赖做 release 构建）：
    ///
    /// | 调用（1 帧输入） | 比例 | 内部处理帧数 | 读到的墙钟 |
    /// | :--- | :--- | :--- | :--- |
    /// | `resample_interleaved(&[0.0], 1, 2, 4_000)` | 2 000 | 2 306 010 | 11.5 s |
    /// | `resample_interleaved(&[0.0], 1, 2, 8_000)` | 4 000 | 4 612 010 | > 20 s（未在 20 s 内返回） |
    ///
    /// ⚠ 读数用的是**比例闸门被关掉**的预算（`max_resample_ratio = u64::MAX`）：本条量的是
    /// "重采样器自己要多少"，那是 `rubato` 的配方，与本 crate 的闸门无关。**默认预算下这
    /// 两行已经到不了 `try_reserve`** —— 第六道闸门（[`limits::check_resample_ratio`]）在
    /// `Async::new_sinc` 之前就把它们拒了；本条末尾与
    /// `the_resample_ratio_cap_bounds_the_resampler_working_set` 一起钉住这一点。
    ///
    /// 注入：把 `Async::new_sinc` 的分块实参从 `CHUNK_FRAMES` 改成 `1` ⇒ 本条第一个读数
    /// 从 `4 612 010` 变成 `520 010`，本判据以 `left: 520010` / `right: 4612010` 红。
    #[test]
    fn the_resampler_demands_chunk_frames_times_ratio_regardless_of_input_length() {
        // 只让时长闸门生效：其余五道放宽到不可能触发（含比例闸门 —— 见上面的 ⚠）。
        let one_second = PcmBudget::new(u64::MAX, u64::MAX, u16::MAX, u32::MAX, 1, u64::MAX);

        // 从错误里读"要求帧数"，而不是跑重采样。
        let read_needed_frames = |frames: usize, channels: u16, in_rate: u32, out_rate: u32| {
            let samples = vec![0.0f32; frames * usize::from(channels)];
            match resample_interleaved_with_budget(
                &samples,
                channels,
                in_rate,
                out_rate,
                &one_second,
            ) {
                Err(DecodeError::Budget(LimitViolation::DurationTooLong { frames, .. })) => frames,
                other => panic!(
                    "the tight duration cap must reject before any resampling: {in_rate} -> \
                     {out_rate} at {frames} frames gave {other:?}"
                ),
            }
        };

        let ratio = 4_000u64; // 8 000 Hz / 2 Hz
        let needed_mono = read_needed_frames(1, 1, 2, 8_000);
        let needed_stereo = read_needed_frames(1, 2, 2, 8_000);
        let chunk = u64::try_from(CHUNK_FRAMES).unwrap();
        let delay_coefficient = u64::try_from(SINC_LEN / 2).unwrap();
        assert_eq!(
            needed_mono,
            chunk * ratio + 10 + delay_coefficient * ratio + ratio
        );
        assert_eq!(needed_mono, 4_612_010);
        // 与声道数无关 —— 这个数描述的是**每声道帧数**。
        assert_eq!(needed_stereo, needed_mono);

        // 与理想输出的比：`limits` 自己给出的理想值是 4 000 帧。
        let contract = limits::resample_len_contract(1, 8_000, 2).expect("non-zero rates");
        assert_eq!(contract.ideal_floor, ratio);
        assert_eq!(needed_mono / contract.ideal_floor, 1_153);

        // 合法参数的**上界**：`DEFAULT_MAX_SAMPLE_RATE` / 1 Hz。
        let widest = read_needed_frames(1, 1, 1, limits::DEFAULT_MAX_SAMPLE_RATE);
        assert_eq!(widest, 885_504_010);
        let widest_bytes = widest * 4;
        assert_eq!(widest_bytes, 3_542_016_040);

        // `check_layout` 自己仍然放行 —— 这道闸门量的是**输出那一份**，而它确实在预算内。
        // 也就是说：`check_layout` 的四道闸门从来没有度量过"比例"，这不是它判错了。
        let default = PcmBudget::default();
        assert_eq!(
            limits::check_layout(1, limits::DEFAULT_MAX_SAMPLE_RATE, widest, &default),
            Ok(())
        );
        assert!(widest * 4 <= default.max_pcm_bytes);
        assert!(widest <= default.max_duration_frames(limits::DEFAULT_MAX_SAMPLE_RATE));

        // 默认预算下**是**另一道闸门（比例）拒绝它，而且判定在任何分配之前 —— 本条的
        // 两个读数都超出了 `DEFAULT_MAX_RESAMPLE_RATIO`。
        assert_eq!(
            default.max_resample_ratio,
            limits::DEFAULT_MAX_RESAMPLE_RATIO
        );
        for (in_rate, out_rate) in [(2u32, 8_000u32), (1, limits::DEFAULT_MAX_SAMPLE_RATE)] {
            assert!(
                matches!(
                    limits::check_resample_ratio(in_rate, out_rate, &default),
                    Err(LimitViolation::ResampleRatioTooHigh { .. })
                ),
                "{in_rate} -> {out_rate} must trip the ratio gate on the default budget"
            );
        }
    }

    /// 判据（类别④ 参数极值 / [ARCH-SEC-003]）：重采样**比例**超出
    /// [`limits::PcmBudget::max_resample_ratio`] 时，`resample_interleaved_with_budget`
    /// 必须在**任何分配之前**拒绝，而且这道闸门要排在采样率闸门**之后**。
    ///
    /// 为什么需要这道闸门：见上一条判据的读数 —— 比例是重采样工作集的唯一放大来源
    /// （`(CHUNK_FRAMES + SINC_LEN/2) × 比例`，与输入帧数无关），而 `check_layout` 的四道
    /// 闸门都没有度量它。
    ///
    /// 实测读数（本机、debug 构建、默认预算 `max_resample_ratio = 1 000`）：
    ///
    /// | 输入帧 | 输入率 → 输出率 | 比例 | 默认预算下的结论 |
    /// | :--- | :--- | :--- | :--- |
    /// | 1 | 768 Hz → 768 kHz | 1 000（**恰好等于上限**） | `Ok`（闭区间通过，输出 1 000 帧） |
    /// | 1 | 767 Hz → 768 kHz | 1 001.3 | `ResampleRatioTooHigh` |
    /// | 1 | 1 Hz → 768 kHz | 768 000 | `ResampleRatioTooHigh`（本条用**闸门函数**钉它：走入口会真去要 3.3 GiB） |
    /// | 1 | 96 kHz → 8 kHz | 0.083 | `Ok`（降采样不受这道闸门约束） |
    /// | 1 | 48 kHz → 2 MHz | — | 仍然是 `SampleRateTooHigh`（采样率闸门在前） |
    ///
    /// "判在任何分配之前"的证据是同一对采样率在**关掉比例闸门**的预算下的读数：
    /// 那时它走到时长闸门，报 `DurationTooLong { frames: 885 504 010 }`（= 3 542 016 040
    /// 字节）—— 也就是说比例闸门是唯一挡在那一份 3.3 GiB 分配之前的东西。
    ///
    /// 注入（实测）：删掉 `resample_interleaved_with_budget` 里那一行
    /// `limits::check_resample_ratio(in_rate, out_rate, budget)?;` ⇒ 本条以
    /// `767 -> 768000 must be refused by the ratio gate, got Ok([…])` 红，读数是
    /// `138 passed / 1 failed`（全库只有这一条钉"入口真的问了闸门"）；把
    /// `check_resample_ratio` 的不等式从 `>` 改成 `>=` ⇒ 本条以
    /// `exactly the ratio cap must pass the ratio gate` 红，读数是
    /// `137 passed / 2 failed`（另一条红的是 `limits` 侧的闭区间判据）。
    #[test]
    fn an_over_the_cap_sample_rate_ratio_is_refused_before_any_resampling() {
        let default = PcmBudget::default();
        let cap = default.max_resample_ratio;
        assert_eq!(cap, limits::DEFAULT_MAX_RESAMPLE_RATIO);

        // ① 闭区间：恰好等于上限的一对必须被放行到下一道判定，并且真的跑完（1 帧 → 1 000 帧）。
        let at_cap = resample_interleaved_with_budget(
            &[0.25f32],
            1,
            768,
            limits::DEFAULT_MAX_SAMPLE_RATE,
            &default,
        )
        .expect("exactly the ratio cap must pass the ratio gate");
        assert_eq!(at_cap.len(), usize::try_from(cap).unwrap());

        // ② 越界一档即拒，而且报的采样率与上限就是调用方给的那两个数。
        match resample_interleaved_with_budget(
            &[0.25f32],
            1,
            767,
            limits::DEFAULT_MAX_SAMPLE_RATE,
            &default,
        ) {
            Err(DecodeError::Budget(LimitViolation::ResampleRatioTooHigh {
                in_rate,
                out_rate,
                limit,
            })) => {
                assert_eq!((in_rate, out_rate, limit), (767, 768_000, cap));
            }
            other => panic!("767 -> 768000 must be refused by the ratio gate, got {other:?}"),
        }

        // ③ 合法参数的**上界**：默认预算拒绝它，而且拒绝发生在任何分配之前 ——
        //    关掉比例闸门的同一对采样率会走到时长闸门，报出那一份 3.3 GiB 的要求。
        let ratio_gate_off = PcmBudget {
            max_duration_secs: 0,
            max_resample_ratio: u64::MAX,
            ..PcmBudget::default()
        };
        match resample_interleaved_with_budget(&[0.25f32], 1, 1, 768_000, &ratio_gate_off) {
            Err(DecodeError::Budget(LimitViolation::DurationTooLong {
                frames,
                sample_rate,
                seconds,
                limit_secs,
            })) => {
                assert_eq!(
                    (frames, sample_rate, seconds, limit_secs),
                    (885_504_010, 768_000, 1_153, 0)
                );
            }
            other => {
                panic!("with the ratio gate off the pair must reach the duration gate: {other:?}")
            }
        }
        //
        // ⚠ 那一对采样率**不**用默认预算走入口判据：闸门一旦被删掉，入口就会真的去要
        // 885 504 010 帧（3.3 GiB）并按比例 768 000 跑 sinc —— 判据会**挂住**而不是变红。
        // 因此极端的比例钉在闸门函数上（不分配），入口判据只用"能真跑完的越界对"
        // （767 Hz → 768 kHz 要 1 154 513 帧 ≈ 4.6 MiB）：删掉闸门后它立刻返回 `Ok`，
        // 判据**快速变红**。
        assert_eq!(
            limits::check_resample_ratio(1, 768_000, &default),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: 1,
                out_rate: 768_000,
                limit: cap
            })
        );
        match resample_interleaved_with_budget(&[0.25f32], 1, 767, 768_000, &default) {
            Err(DecodeError::Budget(LimitViolation::ResampleRatioTooHigh { limit, .. })) => {
                assert_eq!(limit, cap);
            }
            other => panic!("767 -> 768000 must be refused by the ratio gate, got {other:?}"),
        }

        // ④ 降采样不受这道闸门约束（比例 < 1，工作集不放大）。
        assert!(resample_interleaved_with_budget(&[0.25f32], 1, 96_000, 8_000, &default).is_ok());

        // ⑤ 闸门读的是调用方的预算，不是写死的 1 000：同一对采样率在关掉它的预算下能过闸门
        //    （并继续走到下一步），在收紧到 1 的预算下被拒。
        assert!(matches!(
            resample_interleaved_with_budget(&[0.25f32], 1, 48_000, 96_000, &ratio_gate_off),
            Err(DecodeError::Budget(LimitViolation::DurationTooLong { .. }))
        ));
        let tight = PcmBudget {
            max_resample_ratio: 1,
            ..PcmBudget::default()
        };
        assert!(matches!(
            resample_interleaved_with_budget(&[0.25f32], 1, 48_000, 96_000, &tight),
            Err(DecodeError::Budget(LimitViolation::ResampleRatioTooHigh {
                limit: 1,
                ..
            }))
        ));

        // ⑥ 相邻优先级没有被改动：0 率报采样率、超上限的率报采样率、超预算声道数报声道数、
        //    样本数不是声道数的整数倍报布局 —— 这四格在改建前就被拒，报的错误**一字未改**
        //    （新闸门排在它们之后）。
        assert!(matches!(
            resample_interleaved_with_budget(&[0.0; 4], 1, 0, 768_000, &default),
            Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
        ));
        assert!(matches!(
            resample_interleaved_with_budget(&[0.0; 4], 1, 48_000, 2_000_000, &default),
            Err(DecodeError::Budget(
                LimitViolation::SampleRateTooHigh { .. }
            ))
        ));
        // 65 个样本 = 65 声道 × 1 帧：先过"样本数是声道数的整数倍"，再撞声道数闸门。
        assert!(matches!(
            resample_interleaved_with_budget(&[0.0; 65], 65, 1, 768_000, &default),
            Err(DecodeError::Budget(LimitViolation::TooManyChannels { .. }))
        ));
        assert!(matches!(
            resample_interleaved_with_budget(&[0.0; 5], 2, 1, 768_000, &default),
            Err(DecodeError::InconsistentLayout { .. })
        ));

        // ⑦ 0 帧输入的既有语义原样保留：它不消耗任何资源，因此连超比例对也放行。这一格是
        //    新闸门唯一的"例外"，而且方向是**不变**而不是收紧。
        assert!(
            resample_interleaved_with_budget(&[], 1, 1, 768_000, &default)
                .expect("0 frames consumes nothing and stays Ok(empty)")
                .is_empty()
        );
    }

    /// 判据（[ARCH-SEC-003] 的资源上界）：默认预算下，比例闸门把重采样器的**额外工作集**
    /// 钉在 `(CHUNK_FRAMES + SINC_LEN/2) × DEFAULT_MAX_RESAMPLE_RATIO + 10` 帧以内 ——
    /// 也就是那个与输入帧数无关的放大项。
    ///
    /// 量什么：`rubato` 的 `process_all_needed_output_len` 比长度契约的理想输出**多要**的
    /// 帧数（单位：帧）。怎么量：在**关掉比例闸门**的紧时长预算下，从
    /// [`LimitViolation::DurationTooLong`] 的 `frames` 字段读出"要求帧数"，再减去
    /// `limits::resample_len_contract` 给出的 `ideal_ceil`。实测这个差值与输入帧数**无关**
    /// （1 帧与 4 096 帧在同一个比例下是同一个数）。
    ///
    /// 读数（本机 debug 构建）：
    ///
    /// | 比例 | 额外工作集（帧） | 单声道字节 | 64 声道字节 |
    /// | :--- | :--- | :--- | :--- |
    /// | 96（8 kHz → 768 kHz，真实域内最宽） | 110 602 | 442 408 | 28 314 112 |
    /// | 1 000（`DEFAULT_MAX_RESAMPLE_RATIO`，本 crate 的上限） | 1 152 010 | 4 608 040 | 294 914 560 |
    /// | 768 000（1 Hz → 768 kHz，闸门关掉时的无上界情形） | 884 736 010 | 3 538 944 040 | 226 492 418 560 |
    ///
    /// 结论：**3.3 GiB 那一格在默认预算下不可达** —— 比例闸门在 `Async::new_sinc` 之前
    /// 就拒绝它（见上一条判据）。这道闸门只拒绝，不改变任何被放行的输出的位模式。
    ///
    /// 注入（实测）：把 `limits::check_resample_ratio` 的不等式从 `>` 改成 `>=` ⇒ 恰好等于
    /// 上限的那一格（比例 1 000）被误拒，上一条判据以
    /// `exactly the ratio cap must pass the ratio gate` 红（本条自身的算术仍绿 —— 它量的
    /// 是数值，不是判定）；删掉入口那一行调用则**不打红本条**（本条只用闸门函数钉极端
    /// 比例，见上一条判据的 ⚠）。
    #[test]
    fn the_resample_ratio_cap_bounds_the_resampler_working_set() {
        let chunk = u64::try_from(CHUNK_FRAMES).unwrap();
        let delay = u64::try_from(SINC_LEN / 2).unwrap();
        let per_ratio = chunk + delay;
        assert_eq!(per_ratio, 1_152);

        // 关掉比例闸门，只留紧时长闸门 —— 从错误里读"要求帧数"，不真的分配。
        let read_extra = |input_frames: usize, in_rate: u32, out_rate: u32| {
            // 0 秒的时长上限：任何非空输出都被它挡下，因此读到的 `frames` 就是
            // "重采样器要多少"，而比例闸门在这一格是关掉的。
            let probe = PcmBudget {
                max_duration_secs: 0,
                max_resample_ratio: u64::MAX,
                ..PcmBudget::default()
            };
            let samples = vec![0.0f32; input_frames];
            let needed =
                match resample_interleaved_with_budget(&samples, 1, in_rate, out_rate, &probe) {
                    Err(DecodeError::Budget(LimitViolation::DurationTooLong {
                        frames, ..
                    })) => frames,
                    other => panic!(
                        "{input_frames} frames {in_rate} -> {out_rate}: expected DurationTooLong, \
                     got {other:?}"
                    ),
                };
            let ideal = limits::resample_len_contract(input_frames as u64, out_rate, in_rate)
                .expect("non-zero rates")
                .ideal_ceil;
            needed - ideal
        };

        assert_eq!(read_extra(1, 8_000, 768_000), per_ratio * 96 + 10);
        assert_eq!(
            read_extra(1, 1, limits::DEFAULT_MAX_SAMPLE_RATE),
            per_ratio * 768_000 + 10
        );
        assert_eq!(read_extra(1, 8_000, 768_000), 110_602);

        // 放大项与**输入帧数无关**：同一个比例下 1 帧、1 000 帧、4 096 帧读到的是同一个数
        // （2 Hz → 8 kHz 是 4 000×，与 `/tmp` 复现探针的读数逐帧对上）。
        for frames in [1usize, 1_000, 4_096] {
            assert_eq!(
                read_extra(frames, 2, 8_000),
                per_ratio * 4_000 + 10,
                "{frames} frames at ratio 4 000 must carry the same working set"
            );
        }
        assert_eq!(per_ratio * 4_000 + 10, 4_608_010);

        let at_cap = per_ratio * limits::DEFAULT_MAX_RESAMPLE_RATIO + 10;
        assert_eq!(at_cap, 1_152_010);
        assert_eq!(at_cap * 4, 4_608_040);
        assert_eq!(
            at_cap * 4 * u64::from(limits::DEFAULT_MAX_CHANNELS),
            294_914_560
        );

        // 上限之内最宽的那一格仍远小于闸门关掉时的无上界情形。
        let unbounded = per_ratio * 768_000 + 10;
        assert_eq!(unbounded, 884_736_010);
        assert_eq!(unbounded * 4, 3_538_944_040);
        assert!(at_cap < unbounded);

        // 无上界情形的那一对采样率在默认预算下**到不了**分配：比例闸门先拒。
        // 这里量的是闸门函数本身（零分配）—— 走入口的话，闸门被删掉就会真的去要 3.3 GiB。
        assert!(matches!(
            limits::check_resample_ratio(1, limits::DEFAULT_MAX_SAMPLE_RATE, &PcmBudget::default()),
            Err(LimitViolation::ResampleRatioTooHigh { .. })
        ));
    }

    /// 判据（[ARCH-DET-001] 确定性配置的载荷性）：[`CHUNK_FRAMES`] **参与逐位结果**，
    /// 因此"给短输入用更小的分块"不是一条免费的优化。
    ///
    /// 量什么：同一段输入、同一组采样率，两次重采样产出的 `f32` 位模式（`to_bits()`）
    /// 在哪个下标上第一次不同。怎么量：一侧走本 crate 的 [`resample_interleaved`]
    /// （内部固定 `CHUNK_FRAMES`），另一侧用**同一组** rubato 构造参数、只把分块换成
    /// `输入帧数`（下限 1），再逐位比对。
    ///
    /// 读数（`/tmp` 探针 2026-10-10，本机 debug 构建，217 组 `(输入帧数, 比例, 声道数)`
    /// 里 212 组逐位相同、5 组不同）：
    ///
    /// | 输入帧 | 输入率 → 输出率 | 声道 | 输出帧数 | 首个不同的位下标 |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | 1 | 8 kHz → 96 kHz | 1 | 12 | 8 |
    /// | 1 | 8 kHz → 48 kHz | 1 | 6 | 5 |
    /// | 1 | 44.1 kHz → 96 kHz | 1 | 3 | 1 |
    /// | 1 | 44.1 kHz → 48 kHz | 1 | 3 | 无（逐位相同，对照组） |
    ///
    /// 结论：分块**不是**内部实现细节，它是本 crate 发布的确定性口径的一部分；
    /// 上一条判据里那 3.30 GiB 的资源放大因此**不能**用"缩小分块"来修 —— 那会改变输出。
    /// 它现在的落点是**比例闸门**（[`limits::check_resample_ratio`]）：那道闸门只拒绝、
    /// 不碰分块。
    ///
    /// 位模式证据（`/tmp` 位模式矩阵探针，同一份源码对改动前后的两份 `rlib` 各编一次）：
    /// 15 组 `(输入帧数, 声道数, 采样率对)` 里，**改动前后都是 `Ok` 的 11 组逐字节相同**
    /// （逐行 `cmp` 0 处不同，逐行 SHA-256 两两相等），3 组由 `Ok` 变成
    /// `ResampleRatioTooHigh`（比例 1001.3 / 2000 / 1001.3，全部超过上限），1 组由
    /// `DurationTooLong(885 504 010)` 变成 `ResampleRatioTooHigh`（比例 768 000，闸门更早
    /// 就拒了），**没有一组由 `Err` 变成 `Ok`**。
    ///
    /// 注入（实测）：把 `Async::new_sinc` 的分块实参从 `CHUNK_FRAMES` 改成 `1` ⇒ 表里第
    /// 一行变成"无差异"，本判据以
    /// `8000 -> 96000: first bit difference moved` + `left: None` / `right: Some(8)` 红
    /// （同一次注入也让上一条判据的读数从 `4 612 010` 变成 `520 010`）。
    #[test]
    fn the_chunk_size_takes_part_in_the_bit_exact_output() {
        // 与 `resample_interleaved_with_budget` 同一组构造参数，唯一变量是分块。
        let with_chunk = |samples: &[f32],
                          channels: u16,
                          in_rate: u32,
                          out_rate: u32,
                          chunk: usize| {
            let frames = samples.len() / usize::from(channels);
            let ratio = f64::from(out_rate) / f64::from(in_rate);
            let params = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2);
            let mut resampler = Async::<f32>::new_sinc(
                ratio,
                MAX_RELATIVE_RATIO,
                &params,
                chunk,
                usize::from(channels),
                FixedAsync::Input,
            )
            .expect("the same construction the production path uses");
            let needed_frames = resampler.process_all_needed_output_len(frames);
            let mut output: Vec<f32> = Vec::new();
            output.resize(needed_frames * usize::from(channels), 0.0);
            let produced = {
                let input =
                    InterleavedSlice::new(samples, usize::from(channels), frames).expect("input");
                let mut out =
                    InterleavedSlice::new_mut(&mut output, usize::from(channels), needed_frames)
                        .expect("output");
                let (_, produced) = resampler
                    .process_all_into_buffer(&input, &mut out, frames, None)
                    .expect("process_all_into_buffer");
                produced
            };
            output.truncate(produced * usize::from(channels));
            output
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>()
        };

        // (输入帧数, 输入率, 输出率, 期望的首个不同下标；None = 期望逐位相同)
        let rows: [(usize, u32, u32, Option<usize>); 4] = [
            (1, 8_000, 96_000, Some(8)),
            (1, 8_000, 48_000, Some(5)),
            (1, 44_100, 96_000, Some(1)),
            (1, 44_100, 48_000, None),
        ];
        for (frames, in_rate, out_rate, expected_first_diff) in rows {
            let samples: Vec<f32> = (0..frames).map(|i| (i % 61) as f32 * 0.01 - 0.3).collect();
            let published = resample_interleaved(&samples, 1, in_rate, out_rate)
                .expect("the published entry point")
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>();
            let small_chunk = with_chunk(&samples, 1, in_rate, out_rate, frames.max(1));
            assert_eq!(
                published.len(),
                small_chunk.len(),
                "{in_rate} -> {out_rate}: the produced length must not depend on the chunk size"
            );
            let first_diff = published
                .iter()
                .zip(small_chunk.iter())
                .position(|(a, b)| a != b);
            assert_eq!(
                first_diff, expected_first_diff,
                "{in_rate} -> {out_rate}: first bit difference moved"
            );
        }
    }
    /// 判据（类别④ 参数极值）：两个重采样入口在**采样率的 0 端点与类型极大值端点**上
    /// 都必须给类型化错误，且错误里报的采样率就是调用方给的那个（不是被夹过的近似值）。
    ///
    /// 逐项判定（量什么 → 怎么量 → 结论）：
    ///
    /// | 入口 | 输入 | 期望 | 依据 |
    /// | :--- | :--- | :--- | :--- |
    /// | `resample_interleaved` | `in_rate = 0` | `ZeroSampleRate` | 第一道闸门 |
    /// | `resample_interleaved` | `out_rate = 0` | `ZeroSampleRate` | 第一道闸门 |
    /// | `resample_interleaved` | `out_rate = u32::MAX` | `SampleRateTooHigh { rate: u32::MAX, limit: 768 000 }` | 闸门在任何分配之前 |
    /// | `resample_interleaved` | `in_rate = u32::MAX` | `SampleRateTooHigh { rate: u32::MAX, .. }` | `rate = in_rate.max(out_rate)` |
    /// | `resample_asset` | `out_rate = 0` | `ZeroSampleRate` | 落到 `resample_interleaved_with_budget` |
    /// | `resample_asset` | `out_rate = u32::MAX` | `SampleRateTooHigh` | 同上 |
    ///
    /// 量的是 **Hz**。这一格此前没有判据：既有的 `degenerate_arguments_are_rejected_rather_than_guessed`
    /// 用的是 `2_000_000`（超过上限但不是类型极大值），而 `resample_asset` 的 0 端点
    /// 一处都没测。
    ///
    /// 注入（实测）：把 `out_rate > budget.max_sample_rate || in_rate > budget.max_sample_rate`
    /// 里的 `>` 改成 `>=` ⇒ 恰好 `max_sample_rate` 的合法转换被误拒，本条以
    /// `exactly max_sample_rate must not be refused by the rate gate` 红（`0 passed /
    /// 1 failed`，只打红这一条）；`u32::MAX` 那几行仍绿 —— 因此本判据同时钉住
    /// "闸门存在"与"闸门是闭区间"。
    #[test]
    fn the_rate_endpoints_are_refused_and_reported_verbatim() {
        let default_limit = PcmBudget::default().max_sample_rate;
        for (in_rate, out_rate) in [(0u32, 48_000u32), (48_000, 0)] {
            assert!(
                matches!(
                    resample_interleaved(&[0.0; 4], 1, in_rate, out_rate),
                    Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
                ),
                "{in_rate} -> {out_rate} must be a zero-rate error"
            );
        }
        for (in_rate, out_rate) in [(1u32, u32::MAX), (u32::MAX, 1)] {
            match resample_interleaved(&[0.0; 4], 1, in_rate, out_rate) {
                Err(DecodeError::Budget(LimitViolation::SampleRateTooHigh { rate, limit })) => {
                    assert_eq!(rate, in_rate.max(out_rate));
                    assert_eq!(limit, default_limit);
                }
                other => {
                    panic!("{in_rate} -> {out_rate}: expected SampleRateTooHigh, got {other:?}")
                }
            }
        }
        // 闭区间对照：恰好等于上限的一对必须仍然被放行到下一道判定
        // （48 帧 @ 768 kHz → 768 kHz 走恒等路径，代价可忽略）。
        assert!(
            resample_interleaved(&[0.0; 48], 1, default_limit, default_limit).is_ok(),
            "exactly max_sample_rate must not be refused by the rate gate"
        );

        let asset = decode_bytes(&wav_f32(4, 1, 48_000, 0.25), &DecodeOptions::default())
            .expect("a 4-frame 48 kHz mono fixture");
        assert!(
            matches!(
                resample_asset(&asset, 0),
                Err(DecodeError::Budget(LimitViolation::ZeroSampleRate))
            ),
            "the asset entry must not treat a 0 Hz target as the identity path"
        );
        assert!(
            matches!(
                resample_asset(&asset, u32::MAX),
                Err(DecodeError::Budget(LimitViolation::SampleRateTooHigh { rate, .. }))
                    if rate == u32::MAX
            ),
            "the asset entry must refuse an over-cap target rate before resampling"
        );
    }

    /// 判据（待裁决项的判据材料）：**相对（scale-free）闸门不可能**把「1 帧 @ 1 Hz →
    /// 768 kHz」的资源放大与**既有判据已在用**的短片段转换分开 —— 两者的相对放大几乎相同，
    /// 只差**绝对量级**。
    ///
    /// 量什么：`resample_interleaved_with_budget` 在任何分配之前交给
    /// [`limits::check_layout`] 的帧数（单位：帧），以及它相对**理想输出**
    /// （`输入帧数 × 输出率 ÷ 输入率`，精确有理数）的倍数（单位：百万分之一，无量纲）。
    /// 怎么量：把该函数交给一个**时长上限为 0** 的预算，从
    /// [`LimitViolation::DurationTooLong`] 的 `frames` 字段里**读**出来 —— 判定发生在
    /// 分配之前，因此本条不跑重采样，也不烧 CPU。
    ///
    /// 三条读数（本机 aarch64、debug 构建，`/tmp` 探针与下面的断言同值）：
    ///
    /// | 输入 | 理想输出帧数 | 要求帧数 | 放大 |
    /// | :--- | ---: | ---: | ---: |
    /// | 1 帧 2 声道 44.1 kHz → 48 kHz（**既有判据在用**） | 1.088 4… | 1 265 | **1 162.218 75×** |
    /// | 96 000 帧 1 声道 48 kHz → 96 kHz（既有判据在用） | 192 000 | 194 314 | 1.012 052× |
    /// | 1 帧 1 声道 1 Hz → 768 kHz（参数合法上界） | 768 000 | 885 504 010 | **1 153.000 013×** |
    ///
    /// 判定表（每一行的处置）：
    ///
    /// - 第一行由判据
    ///   `identical_channels_resample_bit_identically_and_a_silent_channel_stays_silent`
    ///   钉住（现位于第 1120 行起）：它断言那一路输出里静音声道逐位 `+0.0`、被驱动声道
    ///   非零，因此第一行不是可以随手丢弃的边角料，而是一份**已经被守护的输出位模式**。
    /// - 第二行是本 crate 判据套件里**通过全部预算闸门**的最大要求帧数（把整个套件跑一遍、
    ///   记录通过闸门后的读数得到；同一趟里通过闸门的最大转换比例是 12，来自 8 kHz → 96 kHz）。
    /// - 第三行是 [ARCH-SEC-003] 要求的"不可信输入的资源上限"在此处的缺口：默认预算的
    ///   四道闸门全部放行，见判据
    ///   `the_resampler_demands_chunk_frames_times_ratio_regardless_of_input_length`。
    ///
    /// 结论：第一行的相对放大**大于**第三行（差 9 218 737 ppm，约 0.79%）⇒ 「放大倍数 ≤ K」
    /// 这类闸门会**先**拒掉第一行；第一行与第三行只差**绝对值**（1 265 帧 vs 885 504 010 帧，
    /// 相差 700 003 倍）。⇒ 能把两者分开的闸门只能落在**绝对值**（要求帧数上限）或
    /// **转换比例**（最小输入采样率／最大比例）上，而那两个数值是产品裁决，本 crate 不发明。
    /// 作为对照：第二行是"正常素材"的量级（放大 1.012×，要求 194 314 帧 ≈ 777 KiB）。
    ///
    /// 为什么这条判据本身有用：它把"相对闸门不可能"变成可复算的机械读数，并把任何绝对闸门
    /// 必须落在的**开区间**（194 314, 885 504 010）帧、以及比例闸门必须落在的区间
    /// （12, 768 000）钉在判据里。若日后有人改 `CHUNK_FRAMES` / `SINC_LEN` / `rubato` 的调用
    /// 参数，使得第一行的相对放大**不再大于**第三行，本条会红 —— 那时"放大倍数上限"才第一次
    /// 成为一个可能选项，本判据会提醒重新裁决。
    ///
    /// 注入（实测）：把生产路径的 `SincInterpolationParameters::new(256, …)` 的 `256` 改成
    /// `128` ⇒ 前置延迟项减半，第一行读数从 `1 265` 变成 `1 195`，本条以
    /// `left: 1195` / `right: 1265` 红。同一次注入只打红三条判据（`133 passed / 3 failed`）：
    /// 本条、
    /// `the_resampler_demands_chunk_frames_times_ratio_regardless_of_input_length`
    /// （`left: 4356010` / `right: 4612010`）、
    /// `the_chunk_size_takes_part_in_the_bit_exact_output`
    /// （`first bit difference moved`：`left: Some(0)` / `right: Some(8)`）。
    ///
    /// 注入（实测，第二种）：把生产路径的 `CHUNK_FRAMES` 从 `1024` 改成 `512` ⇒ 分块项减半，
    /// 第一行读数从 `1 265` 变成 `708`，本条以 `left: 708` / `right: 1265` 红；同一次注入
    /// 仍然只打红三条（`133 passed / 3 failed`）：本条、`configuration_is_pinned`
    /// （`left: 512` / `right: 1024`）、
    /// `the_resampler_demands_chunk_frames_times_ratio_regardless_of_input_length`
    /// （`left: 2564010` / `right: 4612010`）。
    #[test]
    fn a_relative_gate_cannot_separate_the_short_clip_amplification_from_a_pinned_conversion() {
        // 时长上限为 0 ⇒ 任何非零要求帧数都被时长闸门拒；判定发生在分配之前。
        let no_time = PcmBudget::new(
            u64::MAX,
            u64::MAX,
            u16::MAX,
            u32::MAX,
            0,
            // ⚠️ 本判据要测**时长**闸门，所以比例闸门必须放开（u64::MAX），
            // 否则新的比例闸门会先触发并返回 RatioTooHigh，测不到时长路径。
            u64::MAX,
        );
        let needed = |frames: usize, channels: u16, in_rate: u32, out_rate: u32| {
            let samples = vec![0.0f32; frames * usize::from(channels)];
            match resample_interleaved_with_budget(&samples, channels, in_rate, out_rate, &no_time)
            {
                Err(DecodeError::Budget(LimitViolation::DurationTooLong { frames, .. })) => frames,
                other => panic!("the zero duration cap must reject before resampling: {other:?}"),
            }
        };
        // 要求帧数相对理想输出帧数的倍数，单位 ppm；全程整数，因此没有浮点误差。
        let amplification_ppm = |needed: u64, frames: u64, in_rate: u32, out_rate: u32| {
            let numerator = u128::from(needed) * u128::from(in_rate) * 1_000_000;
            let denominator = u128::from(frames) * u128::from(out_rate);
            u64::try_from(numerator / denominator).expect("a ppm ratio fits in u64")
        };

        // 第一行：既有判据在用的短片段转换（1 帧、2 声道、44.1 kHz → 48 kHz）。
        let pinned = needed(1, 2, 44_100, 48_000);
        assert_eq!(pinned, 1_265);
        let pinned_ppm = amplification_ppm(pinned, 1, 44_100, 48_000);
        assert_eq!(pinned_ppm, 1_162_218_750);

        // 第二行：通过闸门的最大读数（正常素材量级）。
        let long_input = needed(96_000, 1, 48_000, 96_000);
        assert_eq!(long_input, 194_314);
        let long_ppm = amplification_ppm(long_input, 96_000, 48_000, 96_000);
        assert_eq!(long_ppm, 1_012_052);

        // 第三行：参数合法上界（1 帧、1 声道、1 Hz → 768 kHz）。
        let hole = needed(1, 1, 1, limits::DEFAULT_MAX_SAMPLE_RATE);
        assert_eq!(hole, 885_504_010);
        let hole_ppm = amplification_ppm(hole, 1, 1, limits::DEFAULT_MAX_SAMPLE_RATE);
        assert_eq!(hole_ppm, 1_153_000_013);

        // 相对量级：被守护的那一行放大**更大**，因此"放大倍数上限"型闸门先拒它。
        assert!(
            pinned_ppm > hole_ppm,
            "the pinned conversion ({pinned_ppm} ppm) must outgrow the legal upper bound \
             ({hole_ppm} ppm); otherwise a scale-free amplification cap becomes possible and \
             this decision must be re-adjudicated"
        );
        assert_eq!(pinned_ppm - hole_ppm, 9_218_737);
        // 绝对量级：两者相差 700 003 倍 ⇒ 只有绝对闸门（或比例闸门）能把它们分开。
        assert_eq!(hole / pinned, 700_003);
        // 对照：正常素材的放大是 1.0× 量级。
        assert_eq!(long_ppm / 1_000_000, 1);
    }

    /// 判据（类别② 比率极值 / 六道闸门在两条**恒等**路径上的一致性）：`out_rate ==
    /// in_rate` 时，样本级入口与资产级入口在**全部六道**闸门上给出逐字相同的结论。
    ///
    /// 这是裁决 `R31` 的验收判据（`2026-10-10`）。改建前两条路径在第六道上**相反**：
    /// 样本级报 `ResampleRatioTooHigh`、资产级返回 `Ok`，也就是两个**公开**入口对同一份
    /// 输入、同一份预算给出互相矛盾的答案。
    ///
    /// 量什么：同一份资产、同一个预算下，[`resample_interleaved_with_budget`] 与
    /// [`resample_asset_with_budget`] 的返回值（以及 `DecodeError` 的呈现文本）。
    /// 怎么量：逐道把某一道闸门收紧到必拒（PCM 字节 / 声道数 / 采样率 / 时长），比较两个
    /// 入口的 [`LimitViolation`] 变体与呈现文本；再单独把 `max_resample_ratio` 设成 0 与 1。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 收紧的那道闸门 | 样本级入口 | 资产级入口 |
    /// | :--- | :--- | :--- |
    /// | `max_pcm_bytes = 1` | `PcmBudgetExceeded` | `PcmBudgetExceeded`（文本逐字相同） |
    /// | `max_channels = 1` | `TooManyChannels` | `TooManyChannels`（文本逐字相同） |
    /// | `max_sample_rate = 1` | `SampleRateTooHigh` | `SampleRateTooHigh`（文本逐字相同） |
    /// | `max_duration_secs = 0` | `DurationTooLong` | `DurationTooLong`（文本逐字相同） |
    /// | `max_resample_ratio = 0` | `ResampleRatioTooHigh` | `ResampleRatioTooHigh`（文本逐字相同） |
    /// | `max_resample_ratio = 1` | `Ok` | `Ok` |
    ///
    /// 机制：`resample_interleaved_with_budget` 的第六道闸门排在**恒等分支之前**；
    /// `resample_asset_with_budget` 的恒等分支则在它自己的 `check_layout` 之后补判了**同一条**
    /// 闸门（[`limits::check_resample_ratio`]，位置对齐样本级入口的判定顺序）。两条路径因此
    /// 判的是同一组闸门、同一个闭区间（"比例 1.0 落在 1× 上限内"两边都通过）。
    ///
    /// 注入（实测，反向证明本判据有效）：删掉资产级恒等分支里新增的那一行
    /// `limits::check_resample_ratio(...)` ⇒ 本条的第六道闸门那一格以
    /// `max_resample_ratio: the two identity paths must report the same violation` 红。
    /// 删掉样本级入口的那一行 ⇒ 本条与既有的
    /// `an_over_the_cap_sample_rate_ratio_is_refused_before_any_resampling` 一起红。
    #[test]
    fn the_two_identity_paths_agree_on_all_six_gates() {
        let bytes = wav_f32(4, 2, 48_000, 0.5);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.frame_count(), 4);
        assert_eq!(asset.channels(), 2);
        let samples = asset.samples();
        let channels = asset.channels();
        let rate = asset.sample_rate();

        // 六道闸门：两个入口必须给出同一个变体，而且呈现文本逐字相同。第六道
        // （比例）与其余五道一样进这一张表 —— 这正是裁决 `R31` 要求的一致性。
        let cases: [(&str, PcmBudget); 6] = [
            (
                "max_pcm_bytes",
                PcmBudget {
                    max_pcm_bytes: 1,
                    ..PcmBudget::default()
                },
            ),
            (
                "max_channels",
                PcmBudget {
                    max_channels: 1,
                    ..PcmBudget::default()
                },
            ),
            (
                "max_sample_rate",
                PcmBudget {
                    max_sample_rate: 1,
                    ..PcmBudget::default()
                },
            ),
            (
                "max_duration_secs",
                PcmBudget {
                    max_duration_secs: 0,
                    ..PcmBudget::default()
                },
            ),
            (
                "max_resample_ratio",
                PcmBudget {
                    max_resample_ratio: 0,
                    ..PcmBudget::default()
                },
            ),
            (
                "max_sample_rate_and_ratio_together",
                PcmBudget {
                    max_sample_rate: 1,
                    max_resample_ratio: 0,
                    ..PcmBudget::default()
                },
            ),
        ];
        for (what, budget) in cases {
            let interleaved =
                resample_interleaved_with_budget(samples, channels, rate, rate, &budget)
                    .expect_err("the tightened gate must refuse the sample-level identity path");
            let asset_level = resample_asset_with_budget(&asset, rate, &budget)
                .expect_err("the tightened gate must refuse the asset-level identity path");
            // `DecodeError` 不实现 `PartialEq`（它包裹 `std::io::Error`），因此用
            // `Debug` 的规范呈现比较"变体 + 字段"，再用 `Display` 比较用户可见文本。
            assert_eq!(
                format!("{interleaved:?}"),
                format!("{asset_level:?}"),
                "{what}: the two identity paths must report the same violation"
            );
            assert_eq!(
                interleaved.to_string(),
                asset_level.to_string(),
                "{what}: the two identity paths must render the same text"
            );
        }

        // 第六道闸门的两条路径必须报出**同一个**变体与同一段文本，而且报的数就是调用方
        // 给的那三个（`in_rate == out_rate == 资产采样率`、上限 0）。
        let zero_cap = PcmBudget {
            max_resample_ratio: 0,
            ..PcmBudget::default()
        };
        for (what, entry) in [
            (
                "sample-level",
                resample_interleaved_with_budget(samples, channels, rate, rate, &zero_cap)
                    .map(|out| out.len()),
            ),
            (
                "asset-level",
                resample_asset_with_budget(&asset, rate, &zero_cap)
                    .map(|kept| usize::try_from(kept.frame_count()).unwrap_or(usize::MAX)),
            ),
        ] {
            match entry {
                Err(DecodeError::Budget(LimitViolation::ResampleRatioTooHigh {
                    in_rate,
                    out_rate,
                    limit,
                })) => assert_eq!(
                    (in_rate, out_rate, limit),
                    (rate, rate, 0),
                    "{what}: the reported numbers must be the caller's"
                ),
                other => panic!(
                    "{what}: a 0x ratio cap must refuse an identity pair in BOTH public \
                     entries (裁决 R31), got {other:?}"
                ),
            }
        }

        // 上限恰好 1 时两条路径都放行（比例 1.0 落在闭区间内）。
        let unit_cap = PcmBudget {
            max_resample_ratio: 1,
            ..PcmBudget::default()
        };
        assert_eq!(
            resample_interleaved_with_budget(samples, channels, rate, rate, &unit_cap)
                .expect("ratio 1.0 is inside a 1x cap")
                .len(),
            samples.len()
        );
        assert_eq!(
            resample_asset_with_budget(&asset, rate, &unit_cap)
                .expect("ratio 1.0 is inside a 1x cap")
                .frame_count(),
            asset.frame_count()
        );
    }

    /// 正弦输入（单位：帧数、采样率 Hz、频率 Hz、幅度）。判据用的都是**性质**，因此
    /// 只需一个可复算的波形源，不需要任何跨平台的精确值。
    fn sine(frames: usize, rate: f64, frequency: f64, amplitude: f64) -> Vec<f32> {
        (0..frames)
            .map(|index| {
                let t = index as f64 / rate;
                (amplitude * (std::f64::consts::TAU * frequency * t).sin()) as f32
            })
            .collect()
    }

    /// 均方根（单位：与输入同）。
    fn rms(samples: &[f32]) -> f64 {
        (samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
    }

    /// 判据（类别② 重采样输出位型的**性质**，[ARCH-DSP-002]）：滤波路径必须是
    /// **有限、通带单位增益、且真的抗混叠**的。
    ///
    /// 量什么：1 kHz 正弦（幅度 0.5）经三种采样率转换后的
    /// ① 输出是否全部有限、② 输出/输入 RMS 之比、③ 峰值放大倍数；以及
    /// ④ 一个**高于输出 Nyquist** 的 20 kHz 音经 48 kHz → 24 kHz 之后的剩余 RMS 之比。
    /// 怎么量：手写正弦 ＋ 均方根。**⛔ 不钉任何跨平台精确值**（`ARCH-DET-001` 的 L1/L2
    /// 只要求同平台一致；本判据只钉性质与容差）。
    ///
    /// 读数（本机、debug 构建、4800 帧输入）：
    ///
    /// | 转换 | 输出/输入 RMS | 中段 RMS 比 | 峰值放大 | 全部有限 |
    /// | :--- | ---: | ---: | ---: | :--- |
    /// | 48 kHz → 44.1 kHz | 1.00000 | 1.00000 | 1.00000 | 是 |
    /// | 44.1 kHz → 48 kHz | 0.99995 | 0.99937 | **1.03997** | 是 |
    /// | 48 kHz → 96 kHz | 1.00000 | 1.00000 | 1.00000 | 是 |
    /// | 20 kHz：48 kHz → 24 kHz | **0.00239** | — | — | 是 |
    ///
    /// 容差就是按上表定的（1% 通带增益、6% 峰值、抗混叠 ≤ 5%），因此**不是**在钉精确值。
    ///
    /// 为什么需要它：既有的重采样判据几乎都在量**长度**与**位级一致性**；唯一量"信号性质"
    /// 的是 `no_leading_pad_is_observable_as_an_immediate_half_level_crossing`（直流阶跃的
    /// 0.5 交叉点）。**抗混叠**这条 —— sinc 重采样存在的**唯一理由** —— 此前完全没有判据：
    /// 把滤波器换成"线性插值"或干脆"丢样本"，全部既有判据照旧通过（因为长度与恒等路径
    /// 都不变）。
    ///
    /// 注入（实测）：把 `Async::<f32>::new_sinc(...)` 的窗函数由 `BlackmanHarris2` 改成
    /// `Hamming` 仍然全绿（两者都抗混叠）；要让它红需要**去掉滤波**（例如把
    /// `process_all_into_buffer` 换成"按比例抽点"）—— 那是本判据存在的理由。
    #[test]
    fn the_filtered_path_is_finite_unity_gain_and_anti_aliasing() {
        for (in_rate, out_rate) in [(48_000u32, 44_100u32), (44_100, 48_000), (48_000, 96_000)] {
            let input = sine(4_800, f64::from(in_rate), 1_000.0, 0.5);
            let output = resample_interleaved(&input, 1, in_rate, out_rate)
                .unwrap_or_else(|err| panic!("{in_rate} -> {out_rate}: {err}"));
            assert!(
                output.iter().all(|sample| sample.is_finite()),
                "{in_rate} -> {out_rate}: the filtered path must stay finite"
            );
            let gain = rms(&output) / rms(&input);
            assert!(
                (0.99..=1.01).contains(&gain),
                "{in_rate} -> {out_rate}: a 1 kHz tone is deep inside the passband, so the RMS \
                 gain must be unity within 1%, got {gain}"
            );
            let peak_in = input.iter().fold(0f32, |a, b| a.max(b.abs()));
            let peak_out = output.iter().fold(0f32, |a, b| a.max(b.abs()));
            assert!(
                peak_out <= peak_in * 1.06,
                "{in_rate} -> {out_rate}: the sinc filter's overshoot must stay bounded \
                 (peak {peak_out} vs {peak_in})"
            );
        }

        // 抗混叠：20 kHz 在 48 kHz 里是合法信号，但高于 24 kHz 输出的 Nyquist ⇒ 必须被削掉。
        let aliased = sine(4_800, 48_000.0, 20_000.0, 0.5);
        let attenuated = resample_interleaved(&aliased, 1, 48_000, 24_000)
            .expect("downsampling a 20 kHz tone is legal");
        let remaining = rms(&attenuated) / rms(&aliased);
        assert!(
            remaining <= 0.05,
            "a tone above the output Nyquist must be attenuated by the anti-aliasing filter \
             (at least -26 dB), got a remaining RMS ratio of {remaining} — a resampler that \
             merely drops samples would leave it near 1.0"
        );
        // 对照：同一个 24 kHz 目标上，1 kHz 的音必须几乎原样留下（证明上一条不是"什么都削"）。
        let passband = sine(4_800, 48_000.0, 1_000.0, 0.5);
        let kept = resample_interleaved(&passband, 1, 48_000, 24_000)
            .expect("downsampling a 1 kHz tone is legal");
        let kept_ratio = rms(&kept) / rms(&passband);
        assert!(
            (0.99..=1.01).contains(&kept_ratio),
            "the 1 kHz tone must survive the same conversion, got {kept_ratio}"
        );
    }

    /// 判据（重采样输出位型的**频率/相位性质** [ARCH-DSP-002]）：通带内的正弦经转换之后
    /// **频率不变** —— 用"过零次数"量它（⛔ 不钉任何跨平台精确值）。
    ///
    /// 量什么：1 kHz 正弦经 48 kHz→96 kHz（上采样）与 48 kHz→24 kHz（下采样）之后，
    /// **中段**（去掉两端各 10%，避开滤波器边界）的符号变化次数（单位：次），
    /// 与"频率 1 kHz 应产生的次数"比较。
    /// 怎么量：手写正弦 ＋ 数相邻样本的符号变化。1 kHz 在 `t` 秒内过零 `2 × 1000 × t` 次。
    ///
    /// ⚠ **R77 硬断言普查（本 crate 的机械读数）**：`grep -rnE '\.(sin|cos|exp|ln|log10|powf|powi|tan|sqrt)\('`
    /// 在本 crate 的**全部源码**里只有 **2 处**命中，且都在 `resample.rs` 的**判据辅助函数**里
    /// （`sine()` 的 `.sin()`、`rms()` 的 `.sqrt()`）；`asset.rs` / `decode.rs` / `testfix.rs` /
    /// `limits.rs` / `duration.rs` / `error.rs` / `propcheck.rs` **各 0 处**。
    ///
    /// 但**被调用的依赖**里有超越函数：`resample` 走 rubato 的 sinc 卷积、`decode` 走
    /// symphonia 的 floor0 与 IMDCT（都用 `cos`）。⇒ 本判据断言的是**幅度/次数**，只能
    /// **容差式**（见下），⛔ 不得把任何 DSP 输出的浮点值写成精确相等。
    /// 例外的两类仍然可硬断言：**字节级**夹具（本条不涉及）与**整数计数**
    /// （如 `a_nonzero_residue_spectrum_decodes_to_nonzero_samples` 里的 `nonzero == 128`）——
    /// 计数是精确的，即使算它的路径用了超越函数。**本地全绿不是证据**（冻结架构必然全绿）。
    ///
    /// 读数（本机、debug 构建、4800 帧输入 = 0.1 s）：
    ///
    /// | 转换 | 中段时长 | 过零次数（实测 / 期望） |
    /// | :--- | ---: | :--- |
    /// | 48 kHz→96 kHz | 0.08 s | 160 / 160 |
    /// | 48 kHz→24 kHz | 0.08 s | 160 / 160 |
    ///
    /// 为什么需要它：既有的"信号性质"判据只量**幅度**（直流电平、RMS 增益、抗混叠衰减）。
    /// **频率**是另一条独立性质：一个只会"按比例抽点/插零"的实现可以在幅度上勉强过关，
    /// 但过零次数会立刻错（上采样插零会让过零次数翻倍、下采样抽点会让它减半）。
    ///
    /// 注入（实测）：把上采样分支的 `process_all_into_buffer` 换成"每个输入样本重复两次"
    /// 之外的做法不易注入；本条的可注入面是**中段范围**与**过零计数**：把 `expected` 的系数
    /// 由 `2 * 1000` 改成 `1000` ⇒ 本条红（这正是"频率算错"的可辨形态）。
    #[test]
    fn the_filtered_path_preserves_the_frequency_of_a_passband_tone() {
        /// 相邻样本之间的符号变化次数（跳过精确的 0，避免 DC 段误判）。
        fn zero_crossings(samples: &[f32]) -> usize {
            let mut crossings = 0usize;
            let mut last: Option<bool> = None;
            for sample in samples {
                if *sample == 0.0 {
                    continue;
                }
                let positive = *sample > 0.0;
                if last.is_some_and(|previous| previous != positive) {
                    crossings += 1;
                }
                last = Some(positive);
            }
            crossings
        }
        for (in_rate, out_rate) in [(48_000u32, 96_000u32), (48_000, 24_000)] {
            let input = sine(4_800, f64::from(in_rate), 1_000.0, 0.5);
            let output = resample_interleaved(&input, 1, in_rate, out_rate)
                .unwrap_or_else(|err| panic!("{in_rate} -> {out_rate}: {err}"));
            let frames = output.len();
            // ⚠ R93（非真空断言）：**先**证明被扫集合非空且达到下界。否则一旦 `frames` 为 0
            // （或小到中段为空），`seconds` 与 `expected` 都会是 0，而 `measured` 也是 0
            // ⇒ 下面的容差断言 **`0 <= 2` 真空通过**。本 crate 的这条判据此前就缺这一层。
            assert!(
                frames > 10,
                "{in_rate} -> {out_rate}: a degenerate buffer ({frames} frames) would make the \
                 zero-crossing scan vacuous"
            );
            let lo = frames / 10;
            let hi = frames - frames / 10;
            let middle = &output[lo..hi];
            assert!(
                !middle.is_empty(),
                "{in_rate} -> {out_rate}: the scanned middle must not be empty"
            );
            let seconds = (hi - lo) as f64 / f64::from(out_rate);
            // 一个 1 kHz 正弦每秒过零 2 × 1000 次。
            let expected = 2.0 * 1_000.0 * seconds;
            let measured = zero_crossings(middle) as f64;
            // 下界断言：0.08 s 的 1 kHz 正弦应当有约 160 次过零 ⇒ 远大于 100。
            assert!(
                measured > 100.0,
                "{in_rate} -> {out_rate}: expected a large crossing count over {seconds:.3} s, \
                 measured {measured} — a tiny count means the scan was vacuous"
            );
            assert!(
                (measured - expected).abs() <= 2.0,
                "{in_rate} -> {out_rate}: a 1 kHz tone must stay 1 kHz; expected about \
                 {expected:.0} zero crossings over {seconds:.3} s, measured {measured:.0}"
            );
        }
    }
}
