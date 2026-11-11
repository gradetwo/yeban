//! WAV 读写（PCM16 与 float32，单声道／立体声）。
//!
//! 驱动只写 PCM16 单声道 WAV（导入形状与探针输入一致）；读导出文件时
//! 同时接受 PCM16 与 float32（导出结果是 float32，见设计稿 §3.1 第 9 步）。
//! 解析只认 `RIFF` + `fmt ` + `data` 三段，⛔ 不认扩展块以外的花样。

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::error::OtcpError;

/// 解码出的 WAV。
#[derive(Clone, Debug, PartialEq)]
pub struct WavData {
    /// 通道数。
    pub channels: usize,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 交错采样（f32，-1.0..=1.0）。
    pub samples: Vec<f32>,
}

impl WavData {
    /// 总帧数（每通道）。
    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len().checked_div(self.channels).unwrap_or(0)
    }

    /// 时长（秒）。
    #[must_use]
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }

    /// 取第 `channel` 通道的去交错采样。
    #[must_use]
    pub fn channel(&self, channel: usize) -> Vec<f32> {
        (0..self.frames())
            .map(|frame| self.samples[frame * self.channels + channel])
            .collect()
    }
}

/// 读 WAV 文件（PCM16 或 float32）。
pub fn read_wav(path: &Path) -> Result<WavData, OtcpError> {
    let mut file = File::open(path)
        .map_err(|error| OtcpError::io(format!("读 WAV 失败 {}: {error}", path.display())))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| OtcpError::io(format!("读 WAV 失败 {}: {error}", path.display())))?;
    parse_wav(&bytes, path)
}

/// 解析内存里的 WAV 字节。
pub fn parse_wav(bytes: &[u8], path: &Path) -> Result<WavData, OtcpError> {
    let err = |msg: &str| OtcpError::io(format!("解析 WAV 失败 {}: {msg}", path.display()));
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("不是 RIFF/WAVE 文件"));
    }
    let mut offset = 12usize;
    let mut format: Option<(u16, u16, u32)> = None; // (audio_format, channels, sample_rate)
    let mut data: Option<(usize, usize)> = None; // (start, len)
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| err("块头损坏"))?,
        ) as usize;
        let body = offset + 8;
        if body + size > bytes.len() {
            return Err(err("块超出文件"));
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(err("fmt 块太短"));
                }
                let audio_format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
                let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
                let sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                format = Some((audio_format, channels, sample_rate));
            }
            b"data" => {
                data = Some((body, size));
            }
            _ => {}
        }
        offset = body + size + (size % 2);
    }
    let (audio_format, channels, sample_rate) = format.ok_or_else(|| err("缺 fmt 块"))?;
    let (data_start, data_len) = data.ok_or_else(|| err("缺 data 块"))?;
    if channels == 0 || sample_rate == 0 {
        return Err(err("通道数或采样率为 0"));
    }
    let channels = usize::from(channels);
    let raw = &bytes[data_start..data_start + data_len];
    let samples: Vec<f32> = match audio_format {
        // PCM16。
        1 => {
            if data_len % 2 != 0 {
                return Err(err("PCM16 数据长度不是偶数"));
            }
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| {
                    let value = i16::from_le_bytes(*chunk);
                    f32::from(value) / 32768.0
                })
                .collect()
        }
        // IEEE float32。
        3 => {
            if data_len % 4 != 0 {
                return Err(err("float32 数据长度不是 4 的倍数"));
            }
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect()
        }
        _ => return Err(err(&format!("不支持的编码格式 {audio_format}"))),
    };
    if !samples.len().is_multiple_of(channels) {
        return Err(err("采样数不是通道数的倍数"));
    }
    Ok(WavData {
        channels,
        sample_rate,
        samples,
    })
}

/// 写 PCM16 单声道 WAV。
pub fn write_wav_mono_pcm16(
    path: &Path,
    sample_rate: u32,
    samples: &[f32],
) -> Result<(), OtcpError> {
    let mut file = File::create(path)
        .map_err(|error| OtcpError::io(format!("写 WAV 失败 {}: {error}", path.display())))?;
    let data_len = u32::try_from(samples.len().checked_mul(2).unwrap_or(u32::MAX as usize))
        .unwrap_or(u32::MAX);
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(36u32.wrapping_add(data_len).to_le_bytes().as_slice());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(16u32.to_le_bytes().as_slice());
    header.extend_from_slice(1u16.to_le_bytes().as_slice()); // PCM
    header.extend_from_slice(1u16.to_le_bytes().as_slice()); // mono
    header.extend_from_slice(sample_rate.to_le_bytes().as_slice());
    header.extend_from_slice((sample_rate * 2).to_le_bytes().as_slice()); // byte rate
    header.extend_from_slice(2u16.to_le_bytes().as_slice()); // block align
    header.extend_from_slice(16u16.to_le_bytes().as_slice()); // bits
    header.extend_from_slice(b"data");
    header.extend_from_slice(data_len.to_le_bytes().as_slice());
    file.write_all(&header)
        .map_err(|error| OtcpError::io(format!("写 WAV 失败 {}: {error}", path.display())))?;
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * 32767.0).round() as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    file.write_all(&pcm)
        .map_err(|error| OtcpError::io(format!("写 WAV 失败 {}: {error}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_pcm16_mono() {
        let dir = std::env::temp_dir().join("yeban-otcp-wav-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("roundtrip.wav");
        let samples: Vec<f32> = (0..4800)
            .map(|i| (i as f32 / 4800.0 * 2.0 - 1.0) * 0.5)
            .collect();
        write_wav_mono_pcm16(&path, 48_000, &samples).expect("写出");
        let read = read_wav(&path).expect("读回");
        assert_eq!(read.channels, 1);
        assert_eq!(read.sample_rate, 48_000);
        assert_eq!(read.frames(), 4800);
        assert!((read.duration_secs() - 0.1).abs() < 1e-6);
        for (a, b) in samples.iter().zip(read.samples.iter()) {
            assert!((a - b).abs() < 1.0 / 32768.0 + 1e-6);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_float32_wav() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::to_le_bytes(36 + 8));
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&u32::to_le_bytes(16));
        bytes.extend_from_slice(&u16::to_le_bytes(3));
        bytes.extend_from_slice(&u16::to_le_bytes(1));
        bytes.extend_from_slice(&u32::to_le_bytes(44_100));
        bytes.extend_from_slice(&u32::to_le_bytes(44_100 * 4));
        bytes.extend_from_slice(&u16::to_le_bytes(4));
        bytes.extend_from_slice(&u16::to_le_bytes(32));
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::to_le_bytes(8));
        bytes.extend_from_slice(&f32::to_le_bytes(0.5));
        bytes.extend_from_slice(&f32::to_le_bytes(-0.5));
        let wav = parse_wav(&bytes, Path::new("inline.wav")).expect("解析");
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.sample_rate, 44_100);
        assert_eq!(wav.samples, vec![0.5, -0.5]);
    }

    #[test]
    fn rejects_non_wav() {
        let error =
            parse_wav(b"not a wav file at all........", Path::new("x.wav")).expect_err("应拒");
        assert!(error.message.contains("RIFF"));
    }
}
