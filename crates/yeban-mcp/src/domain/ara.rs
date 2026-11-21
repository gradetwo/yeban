//! ARA 会话工具（`yeban_ara_bind` / `yeban_ara_query` / `yeban_ara_render`）的
//! 纯辅助层：wav 解码 → `PcmData`、f32 立体声 wav 编码。
//!
//! 本模块不碰 `Domain`、不碰插件：解码失败与编码失败都以 `Fault` 返回，
//! 由 `domain/mod.rs` 的 plan / apply 函数决定错误码与响应形状。

use std::path::Path;

use yeban_decode::{DecodeOptions, decode_path};
use yeban_plugin_host::PcmData;

use crate::tools::ErrorCode;

use super::error::Fault;

/// 把 wav 文件解成 ARA 片段 PCM（交织 f32 → 非交错声道列）。
///
/// `yeban-decode` 的 `DecodedAsset::samples()` 是交织 f32；`PcmData` 要
/// 每声道一列，此处按声道数拆列。帧数以 `frame_count()` 为准，样本
/// 不足 ⇒ `IO_ERROR`（解码产物自相矛盾时响亮失败，不静默截断）。
pub fn decode_wav_to_pcm(path: &Path) -> Result<PcmData, Fault> {
    let asset = decode_path(path, &DecodeOptions::default()).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("音频解码失败 {}: {error}", path.display()),
        )
    })?;
    let channels = asset.channels() as usize;
    let frames = asset.frame_count() as usize;
    let samples = asset.samples();
    if channels == 0 || samples.len() < frames.saturating_mul(channels) {
        return Err(Fault::domain(
            ErrorCode::IoError,
            format!(
                "解码产物自相矛盾 {}: 声道 {} 帧 {} 样本 {}",
                path.display(),
                channels,
                frames,
                samples.len()
            ),
        ));
    }
    let mut columns: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); channels];
    for frame in 0..frames {
        for channel in 0..channels {
            columns[channel].push(samples[frame * channels + channel]);
        }
    }
    Ok(PcmData {
        channels: columns,
        sample_rate: f64::from(asset.sample_rate()),
    })
}

/// 把立体声 f32 渲染结果编成 32-bit float WAV（RIFF，格式码 3）。
///
/// 确定性编码：无额外 chunk、无抖动、无时间戳，同一渲染结果逐字节
/// 相同。左右声道不等长 ⇒ `INVALID_PARAMETER_RANGE`（调用方不变量
/// 被破坏时响亮失败，不静默截断到短声道）。
pub fn encode_f32_wav_stereo(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<Vec<u8>, Fault> {
    if left.len() != right.len() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "左右声道不等长（{} vs {}），无法编码 WAV",
                left.len(),
                right.len()
            ),
        ));
    }
    let frames = left.len();
    let (data_bytes, riff_size) = checked_wav_sizes(frames, sample_rate)?;
    let capacity = usize::try_from(data_bytes).map_err(|_| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("WAV 数据段 {data_bytes} 字节超出本机 usize 可表达范围"),
        )
    })?;
    let mut out = Vec::with_capacity(capacity + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    // 调用方已保证 `sample_rate * 8` 不溢出 u32（见 `checked_wav_sizes`）。
    out.extend_from_slice(&(sample_rate * 2 * 4).to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for frame in 0..frames {
        out.extend_from_slice(&left[frame].to_le_bytes());
        out.extend_from_slice(&right[frame].to_le_bytes());
    }
    Ok(out)
}

/// 每帧字节数：2 声道 × 4 字节（f32）。
const BYTES_PER_FRAME: u64 = 2 * 4;
/// `data` chunk 之前的固定头长度（RIFF 12 + `fmt ` 24）。
const RIFF_HEADER_BYTES: u64 = 36;
/// RIFF 的 size 字段是 u32：整个文件受这一上限约束。
const RIFF_SIZE_MAX: u64 = u32::MAX as u64;

/// 校验并算出 `(data 段字节数, RIFF size 字段)`。
///
/// RIFF 用 **u32** 描述 `data` 段长度，`frames * 8` 或 `36 + data` 一旦越过
/// `u32::MAX` 就会被截断，产出头部与内容不符的坏 wav。这里在**分配内存之前**
/// 用 u64 算总长并响亮拒绝（T-FIX-38），避免 4 GiB 静默溢出 + OOM。
fn checked_wav_sizes(frames: usize, sample_rate: u32) -> Result<(u32, u32), Fault> {
    let overflow = |what: &str| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "{what}超出 RIFF u32 可表达范围（帧 {frames}，采样率 {sample_rate}），拒绝编码 WAV"
            ),
        )
    };
    let frames_u64 = u64::try_from(frames).map_err(|_| overflow("帧数"))?;
    let data_bytes = frames_u64
        .checked_mul(BYTES_PER_FRAME)
        .ok_or_else(|| overflow("数据段字节数"))?;
    let riff_size = data_bytes
        .checked_add(RIFF_HEADER_BYTES)
        .ok_or_else(|| overflow("RIFF 总长"))?;
    if riff_size > RIFF_SIZE_MAX {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "WAV 总长 {riff_size} 字节超出 RIFF u32 上限 {RIFF_SIZE_MAX}（帧 {frames}），拒绝编码"
            ),
        ));
    }
    // `byteRate = sample_rate × 2 声道 × 4 字节` 也是 u32 字段，同样不许溢出。
    sample_rate
        .checked_mul(8)
        .ok_or_else(|| overflow("字节率"))?;
    let riff_size = u32::try_from(riff_size).map_err(|_| overflow("RIFF 总长"))?;
    let data_bytes = u32::try_from(data_bytes).map_err(|_| overflow("数据段字节数"))?;
    Ok((data_bytes, riff_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_header_for_small_stereo_buffer() {
        let left = [0.5_f32, -0.5];
        let right = [-0.25_f32, 0.25];
        let wav = encode_f32_wav_stereo(&left, &right, 48_000).expect("编码");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        let riff_size = u32::from_le_bytes(wav[4..8].try_into().expect("4 字节"));
        let data_size = u32::from_le_bytes(wav[40..44].try_into().expect("4 字节"));
        assert_eq!(data_size, 2 * 2 * 4);
        assert_eq!(riff_size, 36 + data_size);
        assert_eq!(wav.len(), 44 + data_size as usize);
    }

    #[test]
    fn rejects_data_section_past_riff_u32_limit() {
        // 只算尺寸、不分配：上限内最后一帧可过，多一帧必须响亮拒绝。
        let max_frames = usize::try_from((RIFF_SIZE_MAX - RIFF_HEADER_BYTES) / BYTES_PER_FRAME)
            .expect("帧数可表达");
        assert!(checked_wav_sizes(max_frames, 48_000).is_ok());
        let fault = checked_wav_sizes(max_frames + 1, 48_000).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert!(format!("{fault:?}").contains("RIFF"));
    }

    #[test]
    fn rejects_byte_rate_overflow() {
        // byteRate 是 u32 字段：采样率高到 ×8 溢出时同样不能静默截断。
        let fault = checked_wav_sizes(1, u32::MAX).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
    }
}
