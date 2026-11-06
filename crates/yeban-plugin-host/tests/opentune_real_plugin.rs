//! 真 OpenTune bundle 验证（JUCE 效果器插件）：扫描、装载、离线处理、参数枚举。
//!
//! 默认 `#[ignore]`：依赖本机外的 bundle，不进 CI。跑法（须 `--nocapture` 看实测数）：
//! `OPENTUNE_VST3_PATH=/home/hatch/.vst3/OpenTune.vst3 cargo test -p yeban-plugin-host --test opentune_real_plugin -- --ignored --nocapture`
//!
//! 覆盖链路：`Host::scan`（`~/.vst3` 默认根列出 OpenTune 为可用效果器）→
//! `Host::load_effect`（验证激活＋`latency_samples` 上报）→ `activate` →
//! `process_audio`（正弦直通与离调正弦的 F0 实测）→ `parameters()`（参数表枚举，
//! 为“无生产参数路径”缺口提供证据）。
//!
//! 单测单实例：OpenTune 是重型神经插件（ONNX Runtime＋双模型），故本文件
//! 只有一个测试函数，全部验证顺序跑完。

use std::path::PathBuf;

use yeban_plugin_host::{Host, PluginInstance};

/// OpenTune bundle 路径：环境变量优先，未设时用默认安装路径。
fn opentune_path() -> String {
    match std::env::var("OPENTUNE_VST3_PATH") {
        Ok(path) if !path.is_empty() => path,
        _ => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hatch".to_owned());
            format!("{home}/.vst3/OpenTune.vst3")
        }
    }
}

/// 峰值（绝对值最大）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |max, sample| max.max(sample.abs()))
}

/// 均方根。
fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 自相关估 F0（窗内去均值，滞后取 80–1000 Hz 对应区间的归一化自相关极大值）。
/// 复制自 `gs1_real_plugin.rs` 的 `estimate_f0`（滞后下限放宽到 80 Hz 以覆盖男声区）。
fn estimate_f0(samples: &[f32], sample_rate: f64, from_s: f64, to_s: f64) -> (f64, f32) {
    let from = (from_s * sample_rate) as usize;
    let to = (to_s * sample_rate).min(samples.len() as f64) as usize;
    if to <= from + 2 {
        return (0.0, 0.0);
    }
    let window = &samples[from..to];
    let window_peak = peak(window);
    let mean = window.iter().sum::<f32>() / window.len() as f32;
    let centered: Vec<f32> = window.iter().map(|sample| sample - mean).collect();
    let energy: f32 = centered.iter().map(|sample| sample * sample).sum();
    if energy <= 0.0 {
        return (0.0, window_peak);
    }
    let min_lag = (sample_rate / 1000.0) as usize;
    let max_lag = (sample_rate / 80.0) as usize;
    let mut best_lag = 0_usize;
    let mut best_corr = f32::NEG_INFINITY;
    for lag in min_lag..=max_lag.min(centered.len() - 1) {
        let corr: f32 = centered[..centered.len() - lag]
            .iter()
            .zip(&centered[lag..])
            .map(|(a, b)| a * b)
            .sum();
        let normalized = corr / energy;
        if normalized > best_corr {
            best_corr = normalized;
            best_lag = lag;
        }
    }
    if best_lag == 0 {
        return (0.0, window_peak);
    }
    (sample_rate / best_lag as f64, window_peak)
}

/// 以半音为单位的音分偏差：`cents = 1200 * log2(f / target)`。
fn cents(off: f64, target: f64) -> f64 {
    1200.0 * (off / target).log2()
}

/// 立体声渲染一段正弦：`seconds` 秒，块长 4096；返回（输入左，输出左，输出右）。
fn render_sine(
    instance: &mut Box<dyn PluginInstance>,
    freq: f64,
    seconds: f64,
    sample_rate: f64,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let total = (seconds * sample_rate) as usize;
    let block = 4096_usize;
    let mut in_l_all = Vec::with_capacity(total);
    let mut out_l_all = Vec::with_capacity(total);
    let mut out_r_all = Vec::with_capacity(total);
    let mut done = 0_usize;
    while done < total {
        let frames = block.min(total - done);
        let in_l: Vec<f32> = (0..frames)
            .map(|i| {
                let t = (done + i) as f64 / sample_rate;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * 0.5
            })
            .collect();
        let in_r = in_l.clone();
        let mut out_l = vec![0.0_f32; frames];
        let mut out_r = vec![0.0_f32; frames];
        instance
            .process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r)
            .expect("处理块");
        in_l_all.extend_from_slice(&in_l);
        out_l_all.extend_from_slice(&out_l);
        out_r_all.extend_from_slice(&out_r);
        done += frames;
    }
    (in_l_all, out_l_all, out_r_all)
}

#[test]
#[ignore = "依赖本机外的真 OpenTune bundle（见文件头跑法），不进 CI"]
fn opentune_scans_loads_and_processes_audio() {
    const SAMPLE_RATE: f64 = 48_000.0;
    let path = opentune_path();
    eprintln!("OpenTune bundle: {path}");

    // ① 扫描：~/.vst3 默认根须列出 OpenTune 为可用效果器。
    let vst3_root = PathBuf::from(&path)
        .parent()
        .expect("bundle 路径须有父目录")
        .to_path_buf();
    let found = Host::scan(std::slice::from_ref(&vst3_root));
    for entry in &found {
        eprintln!(
            "扫描行: path={} vendor={} usable={} error={:?} classes={:?}",
            entry.path,
            entry.vendor,
            entry.usable(),
            entry.error,
            entry
                .classes
                .iter()
                .map(|c| (c.name.clone(), format!("{:?}", c.kind), c.error.clone()))
                .collect::<Vec<_>>()
        );
    }
    let descriptor = found
        .iter()
        .find(|entry| entry.path.ends_with("OpenTune.so"))
        .expect("扫描结果须含 OpenTune.so 行");
    assert!(
        descriptor.usable(),
        "OpenTune 须可用: {:?}",
        descriptor.error
    );
    let class = descriptor
        .classes
        .iter()
        .find(|c| c.name == "OpenTune")
        .expect("类表须含 OpenTune");
    assert_eq!(
        class.kind,
        yeban_plugin_host::PluginKind::Effect,
        "OpenTune 须被探针判为效果器"
    );
    assert!(class.error.is_none(), "OpenTune 类探针须无错误");

    // ② 装载：效果器加载＋验证激活，记录实例上报的延迟。
    let mut instance =
        Host::load_effect(&path, Some("OpenTune")).expect("OpenTune 应能以效果器装载");
    eprintln!("实例名: {}", instance.name());
    eprintln!(
        "装载后 latency_samples（未激活）: {}",
        instance.latency_samples()
    );

    // ③ 参数枚举（诊断用：宿主目前无生产参数路径，此表是缺口的证据）。
    match instance.parameters() {
        Ok(params) => {
            eprintln!("参数表: {} 项", params.len());
            for param in &params {
                let value = instance.parameter_value(param.id);
                eprintln!(
                    "  param id={} title={:?} default={:?} value={:?} flags={}",
                    param.id, param.title, param.default_normalized, value, param.flags
                );
            }
        }
        Err(error) => eprintln!("参数枚举失败: {error:?}"),
    }

    // ④ 激活并直通：440 Hz 正弦 2 秒，输出须非静音、峰值 sane。
    instance.activate(SAMPLE_RATE, 4096).expect("激活");
    eprintln!(
        "激活后 latency_samples: {}（{:.1} ms @ 48 kHz）",
        instance.latency_samples(),
        instance.latency_samples() as f64 / 48.0
    );
    let (in_l, out_l, out_r) = render_sine(&mut instance, 440.0, 2.0, SAMPLE_RATE);
    let out_peak = peak(&out_l);
    let out_rms = rms(&out_l);
    eprintln!(
        "440 Hz 直通: 左声道 peak={out_peak:.4} rms={out_rms:.4} 右声道 peak={:.4}",
        peak(&out_r)
    );
    // 逐样本偏差（直通是逐位透传还是重合成，凭此区分）。
    let worst_dev = in_l
        .iter()
        .zip(out_l.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    eprintln!(
        "440 Hz 直通: 最大逐样本偏差 vs 输入 = {worst_dev:.6}（2 LSB = {:.8}）",
        2.0 / 8_388_608.0
    );
    assert!(
        worst_dev <= 1e-6,
        "默认态 OpenTune 直通须逐位透传（最大偏差 {worst_dev} > 1e-6）"
    );
    assert!(
        out_peak > 0.05,
        "440 Hz 正弦过 OpenTune 后不得静音（peak={out_peak}）"
    );
    assert!(
        out_peak <= 1.5,
        "440 Hz 正弦输出峰值须 sane（peak={out_peak}）"
    );
    let (f0_in, _) = estimate_f0(&out_l, SAMPLE_RATE, 0.5, 1.5);
    eprintln!(
        "440 Hz 直通: 输出 F0 ≈ {f0_in:.1} Hz（目标 440 Hz，偏差 {:+.1} 音分）",
        cents(f0_in, 440.0)
    );
    assert!(
        (400.0..480.0).contains(&f0_in),
        "440 Hz 输入的输出 F0 应 ≈440 Hz，实为 {f0_in:.1} Hz"
    );

    // ⑤ 纠音测量：452 Hz 正弦（≈ A4 上方 +46 音分）5 秒，量输出 F0。
    // 本步只量不判：OpenTune 是 Melodyne 式编辑器插件，其 Capture 流程
    // 可能需要编辑器/GUI 介入；默认态是否纠音是本测试要回答的问题，不是前提。
    // 5 秒长窗排除“采集需要时间才启动”的可能：若 Capture 在数秒后接管，
    // 4–4.9 秒窗的 F0 应已向 440 Hz 移动。
    let (sharp_l, _, _) = render_sine(&mut instance, 452.0, 5.0, SAMPLE_RATE);
    let sharp_peak = peak(&sharp_l);
    eprintln!("452 Hz（+46 音分）输入: 输出 peak={sharp_peak:.4}");
    let (f0_sharp_early, _) = estimate_f0(&sharp_l, SAMPLE_RATE, 0.25, 0.75);
    let (f0_sharp_late, _) = estimate_f0(&sharp_l, SAMPLE_RATE, 1.0, 1.9);
    let (f0_sharp_very_late, _) = estimate_f0(&sharp_l, SAMPLE_RATE, 4.0, 4.9);
    eprintln!(
        "452 Hz 输入: 输出 F0 前窗 ≈ {f0_sharp_early:.1} Hz（{:+.1} 音分 vs 440），中窗 ≈ {f0_sharp_late:.1} Hz（{:+.1} 音分 vs 440），末窗 ≈ {f0_sharp_very_late:.1} Hz（{:+.1} 音分 vs 440）",
        cents(f0_sharp_early, 440.0),
        cents(f0_sharp_late, 440.0),
        cents(f0_sharp_very_late, 440.0)
    );
    eprintln!(
        "纠音判定: 输入 452.0 Hz；若后窗 F0 回到 440 Hz 附近（±10 音分内）则默认态纠音成立，否则默认态未纠音（见 REPORT-opentune-vst3.md）"
    );
    // 默认态不纠音（阻塞项，见 REPORT-opentune-vst3.md）：末窗 F0 仍贴在输入
    // 452 Hz 附近（±15 音分内），未向 440 Hz 移动。直通是逐位透传
    // （④ 步最大偏差 0.0），不是重合成后音高不变。一旦参数/Capture 路径
    // 落地，本断言须改为“末窗 F0 回到 440 Hz ±10 音分内”的纠音断言。
    assert!(
        (cents(f0_sharp_very_late, 452.0)).abs() < 15.0,
        "默认态 OpenTune 应直通（末窗 F0 ≈452 Hz），实为 {f0_sharp_very_late:.1} Hz"
    );
    assert!(
        (cents(f0_sharp_very_late, 440.0)).abs() > 30.0,
        "默认态 OpenTune 不得已纠音（末窗 F0 距 440 Hz 须 >30 音分），实为 {f0_sharp_very_late:.1} Hz"
    );
    instance.deactivate();
}
