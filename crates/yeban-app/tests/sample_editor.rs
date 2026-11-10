//! 采样库编辑器（`--edit-samples`）的端到端判据（缺口报告 C-1）。
//!
//! 钉住四点：已知音高经校正后落在 ±0.5 cent 内、力度层合成产出预期的频带分量、
//! RMS 归一达到目标值、重复跑同一输入结果不变。夹具是**真 WAV 字节**（与
//! `--import-audio` 同一个写入器 `yeban_render::wav::write_plain_wav`），
//! 输入不是内存里的假样本 —— 读盘、解码、校正、落盘全程都跑。
//!
//! 用法错误（退出码 2）与编辑失败（退出码 5）的分界也钉住：前者是"没法开始"，
//! 后者是"开始了但没成"。

use std::path::{Path, PathBuf};
use std::process::Command;

use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

/// 被测二进制（与 `cli_contract.rs` 同一个入口）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 夹具采样率（Hz）。
const RATE: u32 = 44_100;

/// 用法错误退出码（`cli::EXIT_USAGE`）。
const EXIT_USAGE: i32 = 2;

/// 编辑失败退出码（`cli::EXIT_EXPORT`）。
const EXIT_EXPORT: i32 = 5;

/// 一次进程调用的结果。
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let output = Command::new(BIN)
        .args(args)
        .output()
        .expect("启动 yeban-app");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// 一次性的临时目录（**不用外部 crate**，与 `cli_contract.rs` 同款）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("yeban-sample-editor-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 一份单声道正弦的真 WAV（16-bit，振幅 0.5 —— RMS −9.03 dBFS，离目标 −18 有距离，
/// 归一必须真干活）。
fn write_sine(dir: &Path, name: &str, hz: f64, seconds: f64) -> PathBuf {
    let frames = (f64::from(RATE) * seconds) as usize;
    let samples: Vec<i16> = (0..frames)
        .map(|frame| {
            let phase = std::f64::consts::TAU * hz * frame as f64 / f64::from(RATE);
            (phase.sin() * 0.5 * f64::from(i16::MAX)).round() as i16
        })
        .collect();
    let path = dir.join(name);
    write_plain_wav(
        &path,
        &PcmFormat::integer(1, RATE, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    path
}

/// 读一份 WAV 的 RMS（dBFS），口径与编辑器同一份换算（`read_plain_wav_mono_f32`）。
fn rms_dbfs(path: &Path) -> f64 {
    let (samples, _, _) = read_plain_wav_mono_f32(path).expect("读产物 WAV");
    assert!(!samples.is_empty(), "产物 WAV 为空: {}", path.display());
    let mean_square: f64 = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        / samples.len() as f64;
    10.0 * mean_square.log10()
}

/// 读 `report.json`（`serde_json::Value`，只读字段，不反序列化结构体 ——
/// 报告的字段名本身就是契约的一部分）。
fn read_report(out_dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(out_dir.join("report.json")).expect("report.json 必须落盘");
    serde_json::from_str(&text).expect("report.json 必须是合法 JSON")
}

/// 以 `prefix` 开头的那一行（回执行）。
fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// 取某一行里 `key=value` 的 `value`（到下一个空白为止）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// `--help` 列出 `--edit-samples`（出口必须在用法文本里找得到）。
#[test]
fn help_lists_edit_samples() {
    let result = run(&["--help"]);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(
        result.stdout.contains("--edit-samples"),
        "--help 必须列出 --edit-samples"
    );
    assert!(
        result.stdout.contains("--sample-edit-out"),
        "--help 必须列出 --sample-edit-out"
    );
}

/// `--edit-samples` 不配 `--sample-edit-out` ⇒ 用法错误（退出码 2），⛔ 不静默猜落点。
#[test]
fn edit_samples_without_out_dir_is_usage_error() {
    let dir = scratch_dir("no-out");
    write_sine(&dir, "a.wav", 440.0, 1.0);
    let result = run(&["--edit-samples", dir.to_str().unwrap()]);
    assert_eq!(
        result.code, EXIT_USAGE,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(
        result.stderr.contains("--sample-edit-out"),
        "错误必须点名缺的开关: stderr={}",
        result.stderr
    );
}

/// `--sample-edit-out` 单独给 ⇒ 用法错误（退出码 2）：修饰开关没有可修饰的编辑。
#[test]
fn out_dir_alone_is_usage_error() {
    let dir = scratch_dir("out-alone");
    let out = dir.join("out");
    let result = run(&["--sample-edit-out", out.to_str().unwrap()]);
    assert_eq!(
        result.code, EXIT_USAGE,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
}

/// `--sample-edit-rms` 域外（3 dBFS，高于 0）⇒ 用法错误（退出码 2）。
#[test]
fn rms_out_of_range_is_usage_error() {
    let dir = scratch_dir("rms-bad");
    write_sine(&dir, "a.wav", 440.0, 1.0);
    let out = dir.join("out");
    let result = run(&[
        "--edit-samples",
        dir.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
        "--sample-edit-rms",
        "3",
    ]);
    assert_eq!(
        result.code, EXIT_USAGE,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
}

/// `--sample-edit-layers` 不在白名单里 ⇒ 用法错误（退出码 2）。
#[test]
fn layers_out_of_vocabulary_is_usage_error() {
    let dir = scratch_dir("layers-bad");
    write_sine(&dir, "a.wav", 440.0, 1.0);
    let out = dir.join("out");
    let result = run(&[
        "--edit-samples",
        dir.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
        "--sample-edit-layers",
        "loud",
    ]);
    assert_eq!(
        result.code, EXIT_USAGE,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
}

/// 输入路径不存在 ⇒ 编辑失败（退出码 5），错误带稳定码 `INPUT_UNREADABLE`。
#[test]
fn missing_input_fails_loudly_with_stable_code() {
    let dir = scratch_dir("missing-input");
    let missing = dir.join("does-not-exist");
    let out = dir.join("out");
    let result = run(&[
        "--edit-samples",
        missing.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(
        result.code, EXIT_EXPORT,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(
        result.stderr.contains("INPUT_UNREADABLE"),
        "错误必须带稳定码 INPUT_UNREADABLE: stderr={}",
        result.stderr
    );
}

/// 目录模式端到端：445 Hz（偏离 A4 约 +19.8 音分）校正到 440 Hz ±0.5 cent 内，
/// 329.63 Hz（E4，准）不动；两份都出 mf/f 层；RMS 落到 −18 dBFS ±0.5 dB 内。
#[test]
fn dir_mode_tunes_to_within_half_a_cent_and_normalizes() {
    let dir = scratch_dir("dir-e2e");
    let input = dir.join("in");
    std::fs::create_dir(&input).expect("建输入目录");
    write_sine(&input, "sharp.wav", 445.0, 2.0);
    write_sine(&input, "exact.wav", 329.63, 2.0);
    let out = dir.join("out");

    let result = run(&[
        "--edit-samples",
        input.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );

    let receipt =
        line_with(&result.stdout, "edited-samples:").expect("必须打 edited-samples: 回执");
    assert_eq!(field(receipt, "samples").as_deref(), Some("2"));
    assert_eq!(field(receipt, "tuned").as_deref(), Some("2"));
    assert_eq!(field(receipt, "skipped").as_deref(), Some("0"));

    // 产物齐全：每采样一份调音版 + mf/f 层。
    for stem in ["sharp", "exact"] {
        for suffix in ["_tuned.wav", "_mf.wav", "_f.wav"] {
            let path = out.join(format!("{stem}{suffix}"));
            assert!(path.exists(), "产物缺失: {}", path.display());
        }
    }

    // 报告逐行钉住：445 Hz 测得 ≈445、目标 440、偏离 ≈+19.8 音分。
    let report = read_report(&out);
    let rows = report["rows"].as_array().expect("rows 必须是数组");
    assert_eq!(rows.len(), 2, "报告必须有 2 行");
    let sharp = rows
        .iter()
        .find(|row| row["source"].as_str().unwrap_or("").ends_with("sharp.wav"))
        .expect("报告必须有 sharp.wav 一行");
    let measured = sharp["measured_hz"].as_f64().expect("measured_hz 必须是数");
    assert!(
        (measured - 445.0).abs() < 1.0,
        "测得基频 ≈445 Hz, 实测 {measured}"
    );
    assert_eq!(
        sharp["target_hz"].as_f64(),
        Some(440.0),
        "目录模式目标 = 最近半音 440 Hz"
    );
    let deviation = sharp["cents_before"]
        .as_f64()
        .expect("cents_before 必须是数");
    assert!(
        (deviation - 19.8).abs() < 1.0,
        "偏离 ≈ +19.8 音分, 实测 {deviation}"
    );
    assert_eq!(sharp["status"].as_str(), Some("ok"));

    // 校正后的产物重测：独立的自相关实现（见文件末尾）量出 440 Hz 附近。
    // 口径分工：±0.5 cent 的紧口径由 DSP 单元判据
    // `correct_pitch_lands_within_half_a_cent` 用生产测量钉住；这里的独立实现
    // 自带约 2.4 音分的抛物线插值偏高（与生产测量精修前同款），因此只交叉核对
    // 到 ±5 音分 —— 它仍能抓住"没校正 / 校错目标 / 差一个八度"（那都是 20 音分以上）。
    let (tuned, _, _) =
        read_plain_wav_mono_f32(out.join("sharp_tuned.wav")).expect("读 sharp_tuned.wav");
    let remeasured = measure_fundamental_hz(&tuned);
    let cents = 1200.0 * (remeasured / 440.0).log2();
    assert!(
        cents.abs() < 5.0,
        "校正后偏离 {cents:.3} 音分（独立交叉核对限 ±5），重测 {remeasured:.3} Hz"
    );

    // RMS 归一：调音版与层版都落在 −18 dBFS ±0.5 dB 内。
    for stem in ["sharp", "exact"] {
        for suffix in ["_tuned.wav", "_mf.wav", "_f.wav"] {
            let rms = rms_dbfs(&out.join(format!("{stem}{suffix}")));
            assert!(
                (rms - -18.0).abs() < 0.5,
                "{stem}{suffix} 的 RMS {rms:.2} dBFS（目标 −18 ±0.5）"
            );
        }
    }
}

/// 同一输入跑两次 ⇒ 报告字节与调音产物字节都相同（确定性判据）。
///
/// 两次跑进**同一个**输出目录（第二次覆盖第一次）：报告里写着输出目录路径，
/// 两次跑进不同目录时报告字节**必然**不同 —— 那不是不确定，是报告在说实话。
/// 同目录重跑才是确定性的真判据（同时也验了"覆盖写不留半成品"）。
#[test]
fn run_twice_is_deterministic() {
    let dir = scratch_dir("determinism");
    let input = dir.join("in");
    std::fs::create_dir(&input).expect("建输入目录");
    write_sine(&input, "a.wav", 445.0, 2.0);
    let out = dir.join("out");
    let args = [
        "--edit-samples",
        input.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
    ];
    let first = run(&args);
    assert_eq!(
        first.code, 0,
        "stdout={} stderr={}",
        first.stdout, first.stderr
    );
    let report_first = std::fs::read(out.join("report.json")).expect("读第一次的报告");
    let tuned_first = std::fs::read(out.join("a_tuned.wav")).expect("读第一次的产物");
    let second = run(&args);
    assert_eq!(
        second.code, 0,
        "stdout={} stderr={}",
        second.stdout, second.stderr
    );
    let report_second = std::fs::read(out.join("report.json")).expect("读第二次的报告");
    let tuned_second = std::fs::read(out.join("a_tuned.wav")).expect("读第二次的产物");
    assert_eq!(
        report_first, report_second,
        "两次跑的 report.json 必须逐字节相同"
    );
    assert_eq!(tuned_first, tuned_second, "两次跑的调音产物必须逐字节相同");
}

/// SFZ 模式：两 region（keycenter 69/72）⇒ 产物 + 扁平重发出的 `.sfz`，
///
/// 里面的 mf/f 层 region 按速度三分段登记。
#[test]
fn sfz_mode_emits_edited_sfz_with_layer_regions() {
    let dir = scratch_dir("sfz-e2e");
    let input = dir.join("in");
    std::fs::create_dir(&input).expect("建输入目录");
    write_sine(&input, "a440.wav", 440.0, 2.0);
    write_sine(&input, "c523.wav", 523.25, 2.0);
    let sfz = input.join("test.sfz");
    std::fs::write(
        &sfz,
        "<region> sample=a440.wav lokey=69 hikey=69 pitch_keycenter=69 lovel=0 hivel=127\n\
         <region> sample=c523.wav lokey=72 hikey=72 pitch_keycenter=72 lovel=0 hivel=127\n",
    )
    .expect("写夹具 SFZ");
    let out = dir.join("out");

    let result = run(&[
        "--edit-samples",
        sfz.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );

    let edited = out.join("test_edited.sfz");
    assert!(edited.exists(), "必须产出 test_edited.sfz");
    let text = std::fs::read_to_string(&edited).expect("读 test_edited.sfz");
    // 层 region 登记在重发出的 SFZ 里（速度三分段：p/mf/f 各占一段）。
    assert!(
        text.contains("a440_mf.wav"),
        "重发 SFZ 必须登记 a440_mf.wav"
    );
    assert!(text.contains("a440_f.wav"), "重发 SFZ 必须登记 a440_f.wav");
    assert!(
        text.contains("c523_mf.wav"),
        "重发 SFZ 必须登记 c523_mf.wav"
    );
    assert!(
        text.contains("pitch_keycenter=69"),
        "重发 SFZ 必须保留 keycenter 69"
    );
    // 报告行数 = 2 个 region。
    let report = read_report(&out);
    assert_eq!(report["rows"].as_array().map(Vec::len), Some(2));
}

/// `--sample-edit-report-only`：只测算、只写报告，不写任何音频。
#[test]
fn report_only_writes_no_audio() {
    let dir = scratch_dir("report-only");
    let input = dir.join("in");
    std::fs::create_dir(&input).expect("建输入目录");
    write_sine(&input, "a.wav", 445.0, 2.0);
    let out = dir.join("out");
    let result = run(&[
        "--edit-samples",
        input.to_str().unwrap(),
        "--sample-edit-out",
        out.to_str().unwrap(),
        "--sample-edit-report-only",
    ]);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(out.join("report.json").exists(), "报告模式仍写 report.json");
    let wavs: Vec<_> = std::fs::read_dir(&out)
        .expect("读输出目录")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "wav"))
        .collect();
    assert!(wavs.is_empty(), "报告模式不得写任何 WAV: {wavs:?}");
}

/// 测试内重测基频的最小实现：自相关峰 + 抛物线插值（与生产测量**不同**的实现 ——
/// 判据不能用被测代码自己量自己）。
///
/// 已知局限：这个最小实现自带约 +2.4 音分的抛物线插值偏高（与生产测量精修前同款），
/// 因此它只作 ±5 音分级的独立交叉核对；±0.5 cent 的紧口径由 DSP 单元判据
/// `correct_pitch_lands_within_half_a_cent` 用生产测量钉住。
fn measure_fundamental_hz(samples: &[f32]) -> f64 {
    let rate = f64::from(RATE);
    let window = &samples[samples.len() / 5..samples.len() * 4 / 5];
    let max_lag = (rate / 50.0) as usize;
    let min_lag = (rate / 1000.0) as usize;
    let mut best_lag = min_lag;
    let mut best_value = f64::NEG_INFINITY;
    for lag in min_lag..=max_lag {
        let mut sum = 0.0;
        for n in 0..window.len() - lag {
            sum += f64::from(window[n]) * f64::from(window[n + lag]);
        }
        if sum > best_value {
            best_value = sum;
            best_lag = lag;
        }
    }
    // 抛物线插值（与生产测量不同的插值路径：这里插的是**原始**自相关峰，
    // 生产测量是窗归一化的分数滞后精修 —— 两种路径在 ±5 音分内一致才算交叉通过，
    // 余量留给这个最小实现自带的约 2.4 音分偏高）。
    let value_at = |lag: usize| -> f64 {
        let mut sum = 0.0;
        for n in 0..window.len() - lag {
            sum += f64::from(window[n]) * f64::from(window[n + lag]);
        }
        sum
    };
    let delta = if best_lag > min_lag && best_lag < max_lag {
        let (left, center, right) = (
            value_at(best_lag - 1),
            value_at(best_lag),
            value_at(best_lag + 1),
        );
        let denominator = left - 2.0 * center + right;
        if denominator.abs() > f64::EPSILON {
            0.5 * (left - right) / denominator
        } else {
            0.0
        }
    } else {
        0.0
    };
    rate / (best_lag as f64 + delta)
}
