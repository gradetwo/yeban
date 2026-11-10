//! 音频切片的判据：SFZ 往返（解析＋真实加载）、三变形后的音高／时长、
//! 淡入淡出、CLI 端到端、失败语义、显式切点。
//!
//! ## 这条判据要证的句子
//!
//! > 已知起音的合成音频切出的 region 能被仓库的 SFZ 解析器解析、
//! > 能被真实加载器加载（样本解出声）；shift +1200 音分 ⇒ 音高翻倍、
//! > 时长不变；stretch 2.0 ⇒ 时长翻倍、音高不变；tape 2.0 ⇒ 音高翻倍、
//! > 时长一半；每片首尾都淡入淡出；CLI 一行收据；坏输入大声失败、不留文件。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::audio_slicer::{SliceOptions, SliceTransform, slice_audio_file};
use yeban_app::cli;
use yeban_app::sfz_load::load_sfz_with_instrument;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};
use yeban_sfz::parser::{ParseLimits, parse_text};

const SAMPLE_RATE: u32 = 48_000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-audio-slicer-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 正弦（`freq` Hz、`amplitude` 幅值、`frames` 帧，初相 0）。
fn sine(freq: f32, amplitude: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let phase = 2.0 * std::f32::consts::PI * freq * index as f32 / SAMPLE_RATE as f32;
            amplitude * phase.sin()
        })
        .collect()
}

/// 余弦（初相 π/2：首样本就是满幅 —— 淡入淡出判据用它，淡入淡出
/// 不施加时首样本绝不会是 0）。
fn cosine(freq: f32, amplitude: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let phase = 2.0 * std::f32::consts::PI * freq * index as f32 / SAMPLE_RATE as f32;
            amplitude * phase.cos()
        })
        .collect()
}

/// 三段 440 Hz 突发（起音 0.20 s / 0.70 s / 1.20 s，段间静音），共 1.6 s。
fn burst_audio() -> Vec<f32> {
    let total = (1.6 * SAMPLE_RATE as f32) as usize;
    let mut audio = vec![0.0f32; total];
    for (start_s, len_s) in [(0.20f32, 0.25f32), (0.70, 0.25), (1.20, 0.25)] {
        let start = (start_s * SAMPLE_RATE as f32) as usize;
        let len = (len_s * SAMPLE_RATE as f32) as usize;
        for (offset, sample) in sine(440.0, 0.8, len).iter().enumerate() {
            audio[start + offset] = *sample;
        }
    }
    audio
}

/// 零交叉数测频率（Hz）：跳过首尾各 10%（淡入淡出不在测量区内）。
fn measure_freq_hz(samples: &[f32]) -> f64 {
    let skip = samples.len() / 10;
    let window = &samples[skip..samples.len() - skip];
    let mut crossings = 0usize;
    for pair in window.windows(2) {
        if (pair[0] < 0.0) != (pair[1] < 0.0) {
            crossings += 1;
        }
    }
    let secs = window.len() as f64 / f64::from(SAMPLE_RATE);
    crossings as f64 / (2.0 * secs)
}

/// 写单声道 32 位浮点 WAV（夹具用）。
fn write_test_wav(path: &Path, samples: &[f32]) {
    let format = PcmFormat::float(1, SAMPLE_RATE, 32);
    write_plain_wav(
        path,
        &format,
        &yeban_render::dither::PcmBuffer::Float32(samples.to_vec()),
    )
    .expect("写夹具 WAV");
}

/// 读切片 WAV（本模块只写 32 位浮点，读回只认这一形）。
fn read_slice_wav(path: &Path) -> Vec<f32> {
    let (_, buffer) = read_plain_wav(path).expect("解码切片 WAV");
    match buffer {
        yeban_render::dither::PcmBuffer::Float32(samples) => samples,
        _ => panic!("切片 WAV 应是 32 位浮点: {}", path.display()),
    }
}

/// 判据（**region 能被解析并加载**）：三段突发 ⇒ 3 片；SFZ 文本经仓库
/// 解析器解析出 3 个 region；真实加载器加载成功、报告 3 个 region、
/// 探测音符 60 能读到能量（样本真的解出来了，不是空壳）。
#[test]
fn sliced_regions_parse_and_load() {
    let scratch = Scratch::new("roundtrip");
    let input = scratch.dir.join("vocal.wav");
    write_test_wav(&input, &burst_audio());
    let out_dir = scratch.dir.join("slices");

    let report = slice_audio_file(&input, &out_dir, &SliceOptions::default())
        .expect("三段突发必须切出 3 片");
    assert_eq!(report.slices, 3, "三段突发 ⇒ 3 片");
    let dropped = report.dropped_leading_frames;
    assert!(
        dropped.abs_diff(9600) <= 960,
        "第一 onset 在真起音 9600 帧 ±960 帧内，丢弃的首段须如实报：实测 {dropped}"
    );

    let sfz_text = std::fs::read_to_string(&report.sfz_path).expect("读 SFZ");
    let instrument = parse_text(&sfz_text, &ParseLimits::default()).expect("SFZ 须能解析");
    assert_eq!(instrument.regions().len(), 3, "解析出的 region 数 = 3");

    let (load_report, _) = load_sfz_with_instrument(&report.sfz_path, Some(60), SAMPLE_RATE as f32)
        .expect("真实加载器须加载成功");
    assert_eq!(load_report.regions, 3, "加载报告的 region 数 = 3");
    let probe = load_report.probe.expect("探测音符 60 须有读数");
    assert!(probe.frames > 0, "探测音符 60 须写出帧（有匹配区域）");
    assert!(
        probe.energy > 0.0,
        "探测音符 60 须读到能量（样本真的解出来了），实测 {}",
        probe.energy
    );
}

/// 判据（**shift 移调**）：440 Hz 正弦、显式切点 0.25 s ⇒ 两片各 0.25 s；
/// shift +1200 音分后每片 880 Hz、时长不变。
#[test]
fn shift_raises_pitch_an_octave_without_changing_duration() {
    let scratch = Scratch::new("shift");
    let input = scratch.dir.join("tone.wav");
    write_test_wav(&input, &sine(440.0, 0.8, SAMPLE_RATE as usize / 2));
    let out_dir = scratch.dir.join("slices");

    let options = SliceOptions {
        transform: SliceTransform::Shift { cents: 1200.0 },
        cuts: Some(vec![0.25]),
        ..SliceOptions::default()
    };
    let report = slice_audio_file(&input, &out_dir, &options).expect("shift 切片");
    assert_eq!(report.slices, 2, "显式切点 0.25 s ⇒ 2 片");
    for index in 0..2 {
        let samples = read_slice_wav(&out_dir.join(format!("tone_{index:03}.wav")));
        assert_eq!(samples.len(), 12_000, "第 {index} 片时长不变（0.25 s）");
        let freq = measure_freq_hz(&samples);
        assert!(
            (freq - 880.0).abs() < 15.0,
            "第 {index} 片应为 880 Hz，实测 {freq:.1} Hz"
        );
    }
}

/// 判据（**stretch 拉伸**）：同上输入，stretch 2.0 ⇒ 每片 0.50 s、仍是 440 Hz。
#[test]
fn stretch_doubles_duration_without_changing_pitch() {
    let scratch = Scratch::new("stretch");
    let input = scratch.dir.join("tone.wav");
    write_test_wav(&input, &sine(440.0, 0.8, SAMPLE_RATE as usize / 2));
    let out_dir = scratch.dir.join("slices");

    let options = SliceOptions {
        transform: SliceTransform::Stretch { rate: 2.0 },
        cuts: Some(vec![0.25]),
        ..SliceOptions::default()
    };
    let report = slice_audio_file(&input, &out_dir, &options).expect("stretch 切片");
    assert_eq!(report.slices, 2, "显式切点 0.25 s ⇒ 2 片");
    for index in 0..2 {
        let samples = read_slice_wav(&out_dir.join(format!("tone_{index:03}.wav")));
        assert_eq!(samples.len(), 24_000, "第 {index} 片时长翻倍（0.50 s）");
        let freq = measure_freq_hz(&samples);
        assert!(
            (freq - 440.0).abs() < 15.0,
            "第 {index} 片应仍为 440 Hz，实测 {freq:.1} Hz"
        );
    }
}

/// 判据（**tape 变速变调**）：同上输入，tape 2.0 ⇒ 每片 0.125 s、880 Hz。
#[test]
fn tape_doubles_pitch_and_halves_duration() {
    let scratch = Scratch::new("tape");
    let input = scratch.dir.join("tone.wav");
    write_test_wav(&input, &sine(440.0, 0.8, SAMPLE_RATE as usize / 2));
    let out_dir = scratch.dir.join("slices");

    let options = SliceOptions {
        transform: SliceTransform::Tape { rate: 2.0 },
        cuts: Some(vec![0.25]),
        ..SliceOptions::default()
    };
    let report = slice_audio_file(&input, &out_dir, &options).expect("tape 切片");
    assert_eq!(report.slices, 2, "显式切点 0.25 s ⇒ 2 片");
    for index in 0..2 {
        let samples = read_slice_wav(&out_dir.join(format!("tone_{index:03}.wav")));
        assert_eq!(samples.len(), 6000, "第 {index} 片时长一半（0.125 s）");
        let freq = measure_freq_hz(&samples);
        assert!(
            (freq - 880.0).abs() < 15.0,
            "第 {index} 片应为 880 Hz，实测 {freq:.1} Hz"
        );
    }
}

/// 判据（**淡入淡出已施加**）：余弦（首样本满幅 0.8）切一片：
/// 首样本 0、次样本 < 0.01（淡入 4 ms 把它压住）、末样本 0、倒数第 2 样本 < 0.01
/// （淡出 10 ms 把它压住）。不施加淡入淡出时这四个值全是满幅量级。
#[test]
fn slices_start_and_end_with_fades() {
    let scratch = Scratch::new("fades");
    let input = scratch.dir.join("tone.wav");
    write_test_wav(&input, &cosine(440.0, 0.8, SAMPLE_RATE as usize));
    let out_dir = scratch.dir.join("slices");

    let options = SliceOptions {
        cuts: Some(vec![0.5]),
        ..SliceOptions::default()
    };
    let report = slice_audio_file(&input, &out_dir, &options).expect("切一片");
    assert_eq!(report.slices, 2, "显式切点 0.5 s ⇒ 2 片");
    let samples = read_slice_wav(&out_dir.join("tone_000.wav"));
    assert_eq!(samples.len(), 24_000, "第 0 片 0.50 s");
    assert_eq!(samples[0], 0.0, "首样本必须 0（淡入起点）");
    assert!(
        samples[1].abs() < 0.01,
        "次样本须 < 0.01（淡入 4 ms 压住），实测 {}",
        samples[1]
    );
    assert_eq!(samples[samples.len() - 1], 0.0, "末样本必须 0（淡出终点）");
    assert!(
        samples[samples.len() - 2].abs() < 0.01,
        "倒数第 2 样本须 < 0.01（淡出 10 ms 压住），实测 {}",
        samples[samples.len() - 2]
    );
}

/// 判据（**显式切点覆盖检测**）：1.0 s 连续正弦、切点 0.25/0.50/0.75 s ⇒
/// 4 片各 12000 帧、什么都不丢（`droppedLeadingFrames=0`）。
#[test]
fn explicit_cuts_override_detection() {
    let scratch = Scratch::new("cuts");
    let input = scratch.dir.join("tone.wav");
    write_test_wav(&input, &sine(440.0, 0.8, SAMPLE_RATE as usize));
    let out_dir = scratch.dir.join("slices");

    let options = SliceOptions {
        cuts: Some(vec![0.25, 0.5, 0.75]),
        ..SliceOptions::default()
    };
    let report = slice_audio_file(&input, &out_dir, &options).expect("显式切点");
    assert_eq!(report.slices, 4, "3 个切点 ⇒ 4 片");
    assert_eq!(report.dropped_leading_frames, 0, "显式切点模式什么都不丢");
    for index in 0..4 {
        let samples = read_slice_wav(&out_dir.join(format!("tone_{index:03}.wav")));
        assert_eq!(samples.len(), 12_000, "第 {index} 片 0.25 s");
    }
}

/// 判据（**CLI 端到端**）：`--slice-audio` 经 `cli::parse`＋`run_batch`
/// 走通，收据一行 `sliced-audio: … slices=3 …`，文件落盘。
#[test]
fn cli_slices_audio_end_to_end() {
    let scratch = Scratch::new("cli");
    let input = scratch.dir.join("vocal.wav");
    write_test_wav(&input, &burst_audio());
    let out_dir = scratch.dir.join("slices");

    let options = cli::parse(&[
        "--slice-audio".to_owned(),
        input.display().to_string(),
        "--slice-out".to_owned(),
        out_dir.display().to_string(),
        "--headless".to_owned(),
    ])
    .expect("解析 --slice-audio");
    let lines = cli::run_batch(&options).expect("批处理须成功");
    let line = lines
        .iter()
        .find(|line| line.starts_with("sliced-audio:"))
        .unwrap_or_else(|| panic!("缺 sliced-audio: 行:\n{}", lines.join("\n")));
    assert!(line.contains("slices=3"), "收据须写 slices=3：{line}");
    assert!(
        line.contains("transform=none"),
        "缺省不变形，收据须写 transform=none：{line}"
    );
    assert!(out_dir.join("vocal_000.wav").is_file(), "第 0 片落盘");
    assert!(out_dir.join("vocal_002.wav").is_file(), "第 2 片落盘");
    assert!(out_dir.join("vocal.sfz").is_file(), "SFZ 落盘");
}

/// 判据（**失败语义**）：非音频输入 ⇒ 大声失败，且不留任何产物
/// （输出目录都不建，SFZ 更不会指向缺失的 WAV）。
#[test]
fn bad_input_fails_loudly_without_leaving_files() {
    let scratch = Scratch::new("badinput");
    let input = scratch.dir.join("not-audio.wav");
    std::fs::write(&input, b"this is not a wav file at all").expect("写坏输入");
    let out_dir = scratch.dir.join("slices");

    let error = slice_audio_file(&input, &out_dir, &SliceOptions::default())
        .expect_err("非音频输入必须失败");
    let message = error.to_string();
    assert!(
        message.contains("解码"),
        "失败原因须明说解码失败：{message}"
    );
    assert!(!out_dir.join("not-audio.sfz").exists(), "失败时不留 SFZ");
    assert!(
        !out_dir.join("not-audio_000.wav").exists(),
        "失败时不留切片 WAV"
    );
}
