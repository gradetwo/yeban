//! Anatomy 底鼓试听渲染：9 套预设各一击 + 默认预设独奏（4 击）。
//!
//! 用法：`cargo run -p yeban-dsp --locked --example anatomy_kick_demo -- [输出目录]`
//! 缺省输出目录是 `anatomy-kick-demo`（相对当前目录）。
//!
//! 每个文件都是 48 kHz、单声道、16-bit PCM。全部文件共用一个固定的
//! `×0.5` 安全增益（不做逐文件峰值归一化，保留预设之间的相对电平）：
//! 最响的预设（somatic-808-gravity）原始峰值 1.857，×0.5 后 0.928，
//! 其余预设原始峰值 1.000–1.143，×0.5 后 0.500–0.572。文件名前的序号
//! 是预设表顺序（01–06 Groove 原预设，07–09 新增摇滚 / 金属预设）。

use std::env;
use std::fs;
use std::path::PathBuf;

use yeban_dsp::drums::{
    ANATOMY_KICK_PRESETS, AnatomyKickParams, DRUM_SLOTS, DrumHit, DrumKitParams, DrumMachine,
    DrumVoice, KickStyle,
};

/// 采样率（Hz）。
const SAMPLE_RATE: u32 = 48_000;
/// 单击渲染长度：2.0 s（长于最长预设的发声总长 0.9 s + 尾巴）。
const HIT_FRAMES: usize = 96_000;
/// 独奏渲染长度：2.5 s（4 击，间隔 0.5 s）。
const SOLO_FRAMES: usize = 120_000;
/// 固定安全增益（见文件头注释）。
const DEMO_GAIN: f32 = 0.5;

/// 最小 16-bit PCM WAV 写出（单声道、48 kHz）。
fn write_wav(path: &PathBuf, samples: &[f32]) {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + samples.len() * 2) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&((samples.len() * 2) as u32).to_le_bytes());
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let value = (clamped * 32_767.0).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("试听 wav 写出失败");
}

/// 给定 Anatomy 参数渲染一串击（`(start_sample, gain)` 列表）。
fn render_hits(params: AnatomyKickParams, hits: &[(u64, f32)], frames: usize) -> Vec<f32> {
    let mut kit = DrumKitParams::DEFAULT;
    kit.kick_style = KickStyle::Anatomy;
    kit.anatomy = params;
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(SAMPLE_RATE);
    machine.set_params(kit);
    for (start, gain) in hits {
        machine.trigger(DrumHit::new(DrumVoice::Kick, *start, *gain));
    }
    let mut out = vec![0.0f32; frames];
    machine.render(0, &mut out);
    for sample in out.iter_mut() {
        *sample *= DEMO_GAIN;
    }
    out
}

/// 峰值（无量纲）。
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

fn main() {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("anatomy-kick-demo"));
    fs::create_dir_all(&out_dir).expect("输出目录创建失败");

    for (index, preset) in ANATOMY_KICK_PRESETS.iter().enumerate() {
        let samples = render_hits(preset.params, &[(0, 1.0)], HIT_FRAMES);
        let name = format!("{:02}-{}.wav", index + 1, preset.id);
        let path = out_dir.join(&name);
        write_wav(&path, &samples);
        println!(
            "{}: {} / {}，峰值 {:.4}（×0.5 后）",
            index + 1,
            preset.id,
            preset.name_zh,
            peak(&samples)
        );
    }

    // 独奏：默认预设，4 击，间隔 0.5 s。
    let solo = render_hits(
        AnatomyKickParams::DEFAULT,
        &[(0, 1.0), (24_000, 0.8), (48_000, 1.0), (72_000, 0.6)],
        SOLO_FRAMES,
    );
    let solo_path = out_dir.join("10-anatomy-default-solo.wav");
    write_wav(&solo_path, &solo);
    println!(
        "10: anatomy-default-solo（默认预设 4 击：1.0 / 0.8 / 1.0 / 0.6），峰值 {:.4}（×0.5 后）",
        peak(&solo)
    );
}
