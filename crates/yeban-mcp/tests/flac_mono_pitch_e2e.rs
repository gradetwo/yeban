//! B1 端到端判据：**周期信号 FLAC → 解码 → SFZ 装载 → 离线渲染 → 测音高**。
//!
//! ## 来历（B1 裁决 (c) 不复现，2026-11-14 实测钉住）
//!
//! 报告称「单声道 FLAC 播放系统性低约 100 音分」（基线 `e0e0cb2`，ixox 长笛
//! 44,100 Hz 单声道 FLAC：全键区恒定 −91~−101 音分；立体声 FLAC 与单声道
//! float WAV 均在 ±10 音分内）。复现实验（盒上 ffmpeg 夹具，基线 `d22b877b5`）
//! 三段全清：
//!
//! | 段 | 夹具 | 读数（音分，相对 440.00 Hz） |
//! | --- | --- | --- |
//! | 解码（`decode_path` 声道 0 与 `decode_mono_channel0` 两路） | 单声道 FLAC 44.1k/48k、24 位单声道 FLAC、立体声 FLAC、单声道 WAV | 全部 +0.00 |
//! | MCP 渲染（`load_sfz` + `render_sfz_wav`，输出 48,000/44,100） | 同上五份 | −0.05 ~ +0.16 |
//! | 引擎渲染（`render_note_into`，WAV 双份 PCM） | 44.1k/48k 样本 × 两引擎率 | −1.23 ~ +0.61（仅跨率重采样对非零） |
//!
//! 全矩阵最大 |偏差| = 1.23 音分，与宣称的约 100 音分差两个数量级，且无单声道
//! 或 FLAC 特异性 ⇒ 偏差不在夜半解码与播放链路，指向 ixox 库本身（其 `.sfz`
//! 的 `pitch_keycenter`/`tune` 声明或样本真实调律）。本判据防未来回归：宣称的
//! 故障量级约 100 音分，门限取 **10 音分**（10 倍余量；本路径实测 ≤0.16 音分）。
//!
//! ## 夹具自包含
//!
//! FLAC 字节由本文件内的极小编码器生成（VERBATIM 子帧 + STREAMINFO +
//! CRC-8/CRC-16，帧长钉 256 样本、总样本数取帧长整数倍），WAV 双份同源
//! 同值。不依赖外部文件、ffmpeg 或环境变量。引擎侧的同一判据在
//! `yeban-engine` 的 `flac_mono_pitch_render` 测试里（PCM 与本文件 FLAC 的
//! 解码输出逐值相同）。
//!
//! ## 判定量的口径
//!
//! - **量**：基频（Hz），由稳态段内**上升零交叉**（线性插值定分数时刻）的平均
//!   周期换算；报告量为相对 440.00 Hz 的音分偏差 `1200 * log2(f / 440)`。
//! - **解码段**窗口 0.5 s..1.5 s；**渲染段**窗口 0.02 s..0.18 s（工具渲染走工程
//!   音符，`filled_project` 的音高 60 音符只有约 0.2 s 发声段，窗必须落在其中）。
//! - 渲染段 SFZ 用 `key=60 pitch_keycenter=60`、演奏音高 60（音高比 = 1 的不动
//!   点，与 69 等价）；并断言 `notesUsingFileSample == 1`，防止把正弦回退当样本。

use std::path::{Path, PathBuf};

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;

// ─── 量测 ────────────────────────────────────────────────────────────────

/// 上升零交叉 + 线性插值求平均基频，返回 (Hz, 相对 440 Hz 的音分)。
fn measure_freq_cents(pcm: &[f32], rate: u32, from_s: f64, to_s: f64) -> (f64, f64) {
    let start = (from_s * f64::from(rate)) as usize;
    let end = ((to_s * f64::from(rate)) as usize).min(pcm.len());
    assert!(end > start + 2, "量测窗口为空");
    let mut crossings: Vec<f64> = Vec::new();
    for i in start..end - 1 {
        let (a, b) = (f64::from(pcm[i]), f64::from(pcm[i + 1]));
        if a < 0.0 && b >= 0.0 {
            crossings.push(i as f64 + (-a / (b - a)));
        }
    }
    assert!(
        crossings.len() >= 40,
        "交叉点太少（{}），信号可能不是 440 Hz 正弦",
        crossings.len()
    );
    let span = crossings[crossings.len() - 1] - crossings[0];
    let freq = (crossings.len() - 1) as f64 / span * f64::from(rate);
    (freq, 1200.0 * (freq / 440.0).log2())
}

// ─── FLAC 极小编码器（VERBATIM 子帧） ────────────────────────────────────

/// MSB-first 位写入器（FLAC 字段是位打包的）。
struct BitWriter {
    bytes: Vec<u8>,
    acc: u8,
    nbits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    fn push_bits(&mut self, value: u64, count: u32) {
        for i in (0..count).rev() {
            let bit = ((value >> i) & 1) as u8;
            self.acc = (self.acc << 1) | bit;
            self.nbits += 1;
            if self.nbits == 8 {
                self.bytes.push(self.acc);
                self.acc = 0;
                self.nbits = 0;
            }
        }
    }

    fn align(&mut self) {
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            self.push_bits(0, u32::from(pad));
        }
    }

    fn into_bytes(mut self) -> Vec<u8> {
        self.align();
        self.bytes
    }
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// FLAC 的 UTF-8 编码数（帧号用；与 `yeban-decode` 的 testfix 同算法，
/// 在 127/128 与 2047/2048 边界上与 UTF-8 自身的编码结果相同）。
fn push_utf8_number(w: &mut BitWriter, value: u64) {
    let bits = 64 - value.leading_zeros();
    let (len, prefix, first_bits) = match bits {
        0..=7 => (1u32, 0x00u8, 7u32),
        8..=11 => (2, 0xC0, 5),
        12..=16 => (3, 0xE0, 4),
        17..=21 => (4, 0xF0, 3),
        22..=26 => (5, 0xF8, 2),
        27..=31 => (6, 0xFC, 1),
        other => panic!("FLAC UTF-8 数最多载 31 位，得到 {other}"),
    };
    let first = prefix
        | u8::try_from((value >> (6 * (len - 1))) & ((1u64 << first_bits) - 1))
            .expect("首字节载荷必入 u8");
    w.push_bits(u64::from(first), 8);
    for index in (0..len - 1).rev() {
        let continuation =
            0x80 | u8::try_from((value >> (6 * index)) & 0x3F).expect("续字节载荷必入 u8");
        w.push_bits(u64::from(continuation), 8);
    }
}

/// 一帧 FLAC（VERBATIM 子帧）。`interleaved` 是按采样点交织的多声道样本，
/// 每个样本是 bps 位有符号整数；帧长钉 256 样本（帧头块长字段只有 8 位，
/// `blocksize-1` 必须 ≤255 —— 与 `yeban-decode` 的 testfix 同口径）。
fn flac_frame(index: u64, interleaved: &[i64], channels: u16, bps: u16) -> Vec<u8> {
    let block = (interleaved.len() / usize::from(channels)) as u64;
    assert_eq!(block, 256, "帧长钉 256");
    let mut header = BitWriter::new();
    header.push_bits(0b11_1111_1111_1110, 14); // 帧同步
    header.push_bits(0, 1); // 保留
    header.push_bits(0, 1); // 块策略 = 固定块长（尾随的编码数是帧号）
    header.push_bits(0b0110, 4); // 块长码：尾随 8 位 (blocksize-1)
    header.push_bits(0b0000, 4); // 采样率码：取自 STREAMINFO
    let channel_code: u64 = match channels {
        1 => 0b0000,
        2 => 0b0001, // 左右独立声道
        _ => panic!("只支持 1/2 声道"),
    };
    header.push_bits(channel_code, 4);
    header.push_bits(0b000, 3); // 位深码：取自 STREAMINFO
    header.push_bits(0, 1); // 保留
    push_utf8_number(&mut header, index); // FLAC 的 UTF-8 编码帧号
    header.push_bits(block - 1, 8);
    header.align();
    let mut frame = header.into_bytes();
    frame.push(crc8(&frame));

    let mut subframes = BitWriter::new();
    for ch in 0..usize::from(channels) {
        subframes.push_bits(0b0000_0010, 8); // pad 0 + VERBATIM(000001) + 无 wasted bits
        for n in 0..block as usize {
            let v = interleaved[n * usize::from(channels) + ch];
            let masked = (v as u64) & ((1u64 << bps) - 1);
            subframes.push_bits(masked, u32::from(bps));
        }
    }
    subframes.align();
    frame.extend_from_slice(&subframes.into_bytes());
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

fn stream_info(
    rate: u32,
    channels: u16,
    bps: u16,
    total_samples: u64,
    min_frame: usize,
    max_frame: usize,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.push_bits(256, 16); // min blocksize
    w.push_bits(256, 16); // max blocksize
    w.push_bits(min_frame as u64, 24); // min framesize（真实帧长，与 testfix 同口径）
    w.push_bits(max_frame as u64, 24); // max framesize
    w.push_bits(u64::from(rate), 20);
    w.push_bits(u64::from(channels - 1), 3);
    w.push_bits(u64::from(bps - 1), 5);
    w.push_bits(total_samples, 36);
    w.align();
    for _ in 0..16 {
        w.push_bits(0, 8); // 全零 MD5 = 未计算（合法）
    }
    let bytes = w.into_bytes();
    assert_eq!(bytes.len(), 34, "STREAMINFO 必须恰 34 字节");
    bytes
}

/// 生成一份 440.00 Hz 正弦 FLAC 字节（振幅 0.75 满程）。
/// `total_samples` 必须是 256 的整数倍。
fn sine_flac(rate: u32, channels: u16, bps: u16, total_samples: usize) -> Vec<u8> {
    assert_eq!(total_samples % 256, 0);
    let max = (1i64 << (bps - 1)) - 1;
    let samples: Vec<i64> = (0..total_samples)
        .map(|n| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / f64::from(rate);
            (0.75 * max as f64 * phase.sin()).round() as i64
        })
        .collect();
    // 先构造全部帧，才能填 STREAMINFO 里的 min/max framesize（与 testfix 同序）。
    let mut encoded_frames: Vec<Vec<u8>> = Vec::new();
    for (fi, chunk) in samples.chunks(256).enumerate() {
        let interleaved: Vec<i64> = chunk
            .iter()
            .flat_map(|&v| (0..channels).map(move |_| v))
            .collect();
        encoded_frames.push(flac_frame(fi as u64, &interleaved, channels, bps));
    }
    let min_frame = encoded_frames.iter().map(Vec::len).min().unwrap_or(0);
    let max_frame = encoded_frames.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC");
    out.push(0x80); // last-block 标记 + 类型 0 (STREAMINFO)
    out.extend_from_slice(&34u32.to_be_bytes()[1..]); // 24 位长度
    out.extend_from_slice(&stream_info(
        rate,
        channels,
        bps,
        total_samples as u64,
        min_frame,
        max_frame,
    ));
    for frame in &encoded_frames {
        out.extend_from_slice(frame);
    }
    out
}

/// 生成 16 位单声道 WAV 字节（与 `sine_flac(…, 1, 16, …)` 同值）。
fn sine_wav(rate: u32, total_samples: usize) -> Vec<u8> {
    let data_len = (total_samples * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for n in 0..total_samples {
        let phase = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / f64::from(rate);
        let v = (0.75 * 32_767.0 * phase.sin()).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

// ─── 渲染脚手架（照 sfz_sample_files.rs 的形状） ──────────────────────────

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-flac-pitch-e2e-{}.yeban", std::process::id())),
            filled_project(),
            false,
        )
        .expect("注入规范工程");
    dispatcher
}

fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 读回渲染 WAV（24 位立体声），返回 (声道数, 采样率, 交织 i32 样本)。
fn wav_read(bytes: &[u8]) -> (u16, u32, Vec<i32>) {
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut cursor = 12usize;
    let mut channels = 0u16;
    let mut rate = 0u32;
    let mut samples: Vec<i32> = Vec::new();
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let size =
            u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().expect("4")) as usize;
        let body = cursor + 8;
        match fourcc {
            b"fmt " => {
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().expect("2"));
                rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().expect("4"));
            }
            b"data" => {
                let end = (body + size).min(bytes.len());
                let mut at = body;
                while at + 3 <= end {
                    samples.push(i32::from_le_bytes([
                        bytes[at],
                        bytes[at + 1],
                        bytes[at + 2],
                        if bytes[at + 2] & 0x80 != 0 {
                            0xff
                        } else {
                            0x00
                        },
                    ]));
                    at += 3;
                }
                break;
            }
            _ => {}
        }
        cursor = body + size + (size % 2);
    }
    (channels, rate, samples)
}

// ─── 判据本体 ────────────────────────────────────────────────────────────

struct Fixture {
    name: &'static str,
    bytes: Vec<u8>,
    rate: u32,
    channels: u16,
    frames: usize,
}

fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "tone44_mono.flac",
            bytes: sine_flac(44_100, 1, 16, 86_016),
            rate: 44_100,
            channels: 1,
            frames: 86_016,
        },
        Fixture {
            name: "tone48_mono.flac",
            bytes: sine_flac(48_000, 1, 16, 94_208),
            rate: 48_000,
            channels: 1,
            frames: 94_208,
        },
        Fixture {
            name: "tone44_mono24.flac",
            bytes: sine_flac(44_100, 1, 24, 86_016),
            rate: 44_100,
            channels: 1,
            frames: 86_016,
        },
        Fixture {
            name: "tone44_stereo.flac",
            bytes: sine_flac(44_100, 2, 16, 86_016),
            rate: 44_100,
            channels: 2,
            frames: 86_016,
        },
        Fixture {
            name: "tone44_mono.wav",
            bytes: sine_wav(44_100, 86_016),
            rate: 44_100,
            channels: 1,
            frames: 86_016,
        },
    ]
}

fn test_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("yeban-flac-pitch-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn write_fixture(dir: &Path, fx: &Fixture) -> PathBuf {
    let path = dir.join(fx.name);
    std::fs::write(&path, &fx.bytes).expect("写夹具");
    path
}

#[test]
fn decode_side_pitch_is_exact() {
    let dir = test_dir("decode");
    for fx in fixtures() {
        let path = write_fixture(&dir, &fx);

        let asset = yeban_decode::decode_path(&path, &yeban_decode::DecodeOptions::default())
            .expect("decode_path");
        assert_eq!(asset.sample_rate(), fx.rate, "{} 采样率", fx.name);
        assert_eq!(asset.channels(), fx.channels, "{} 声道数", fx.name);
        let stride = usize::from(asset.channels().max(1));
        assert_eq!(
            asset.samples().len() / stride,
            fx.frames,
            "{} 帧数",
            fx.name
        );
        let ch0: Vec<f32> = asset.samples().iter().step_by(stride).copied().collect();
        let (_, cents) = measure_freq_cents(&ch0, fx.rate, 0.5, 1.5);
        assert!(
            cents.abs() <= 5.0,
            "{} decode_path 声道 0 偏差 {cents:+.2} 音分（门限 5）",
            fx.name
        );

        // 生产装载路的实际入口（SFZ 样本装载走这一支）。
        let mono =
            yeban_decode::decode_mono_channel0(&path, &yeban_decode::DecodeOptions::default())
                .expect("decode_mono_channel0");
        assert_eq!(mono.sample_hz, fx.rate, "{} mono 采样率", fx.name);
        assert_eq!(mono.pcm.len(), fx.frames, "{} mono 帧数", fx.name);
        let (_, cents) = measure_freq_cents(&mono.pcm, fx.rate, 0.5, 1.5);
        assert!(
            cents.abs() <= 5.0,
            "{} decode_mono_channel0 偏差 {cents:+.2} 音分（门限 5）",
            fx.name
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rendered_pitch_is_exact_at_both_output_rates() {
    let dir = test_dir("render");
    for fx in fixtures() {
        write_fixture(&dir, &fx);
        let sfz = dir.join(format!("{}.sfz", fx.name));
        std::fs::write(
            &sfz,
            format!("<region> sample={} key=60 pitch_keycenter=60\n", fx.name),
        )
        .expect("写 SFZ");
        for out_rate in [48_000u32, 44_100] {
            let mut dispatcher = dispatcher_with_project();
            let loaded = call_tool(
                &mut dispatcher,
                "yeban_load_sfz",
                serde_json::json!({ "path": sfz.display().to_string() }),
            );
            assert_eq!(loaded["status"], "success", "{} 装载: {loaded}", fx.name);
            let target = dir.join(format!("{}-out{out_rate}.wav", fx.name));
            let rendered = call_tool(
                &mut dispatcher,
                "yeban_render_sfz_wav",
                serde_json::json!({
                    "instrumentId": 1,
                    "note": 60,
                    "frames": out_rate * 2,
                    "sampleRate": out_rate,
                    "path": target.display().to_string(),
                }),
            );
            assert_eq!(
                rendered["status"], "success",
                "{}@{out_rate} 渲染: {rendered}",
                fx.name
            );
            assert_eq!(
                rendered["data"]["notesScheduled"].as_u64(),
                Some(1),
                "{}@{out_rate} 必须排上 1 个音符",
                fx.name
            );
            assert_eq!(
                rendered["data"]["notesUsingFileSample"].as_u64(),
                Some(1),
                "{}@{out_rate} 必须真用文件样本（⛔ 不是正弦回退）",
                fx.name
            );
            let bytes = std::fs::read(&target).expect("读回渲染 WAV");
            let (channels, rate, samples) = wav_read(&bytes);
            assert_eq!(
                (channels, rate),
                (2, out_rate),
                "{}@{out_rate} WAV 头",
                fx.name
            );
            let left: Vec<f32> = samples
                .iter()
                .step_by(2)
                .map(|s| *s as f32 / 8_388_608.0)
                .collect();
            let (_, cents) = measure_freq_cents(&left, rate, 0.02, 0.18);
            assert!(
                cents.abs() <= 10.0,
                "{}@{out_rate} 渲染偏差 {cents:+.2} 音分（门限 10；B1 宣称约 −100）",
                fx.name
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
