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
/// # Errors
///
/// - 采样率为 0 或超出 [`PcmBudget::max_sample_rate`]；
/// - `samples.len()` 不是 `channels` 的整数倍；
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
    let frames = samples.len() / channels_usize;
    if frames == 0 {
        return Ok(Vec::new());
    }
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
        let ideal_only = PcmBudget::new(u64::MAX, 44_100 * 4, 64, 768_000, 60);
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
        let roomy = PcmBudget::new(u64::MAX, 8 * 1024 * 1024, 64, 768_000, 60);
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
                &PcmBudget::new(u64::MAX, 1 << 30, 64, 44_100, 60)
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
        let by_time = PcmBudget::new(u64::MAX, !3u64, 64, 192_000, 1);
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
        let tiny = PcmBudget::new(u64::MAX, 8, 1, 96_000, 60);
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
        let one_sample_short = PcmBudget::new(u64::MAX, (samples - 1) * 4, 64, 96_000, 60);
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
        let exact = PcmBudget::new(u64::MAX, samples * 4, 64, 96_000, 60);
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
        let small = PcmBudget::new(u64::MAX, 12_000 * 2 * 4, 64, 96_000, 60);
        assert!(matches!(
            resample_asset_with_budget(&asset, 96_000, &small),
            Err(DecodeError::Budget(_))
        ));
        let converted = resample_asset_with_budget(&asset, 96_000, &PcmBudget::default()).unwrap();
        assert_eq!(converted.sample_rate(), 96_000);
        assert!(converted.frame_count() >= 24_000);
    }
}
