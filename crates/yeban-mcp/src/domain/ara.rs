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
    let data_bytes = frames * 2 * 4;
    let mut out = Vec::with_capacity(44 + data_bytes);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36u32 + data_bytes as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2 * 4).to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for frame in 0..frames {
        out.extend_from_slice(&left[frame].to_le_bytes());
        out.extend_from_slice(&right[frame].to_le_bytes());
    }
    Ok(out)
}
