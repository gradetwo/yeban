//! 真 GS1 bundle 验证（nih-plug 插件）：装载、离线渲染出声、预制切换。
//!
//! 默认 `#[ignore]`：依赖本机外的 bundle，不进 CI。跑法（须 `--nocapture` 看实测数）：
//! `GS1_VST3_PATH=/home/hatch/workspace/synth/.work/dist/GS1.vst3 cargo test -p yeban-plugin-host --test gs1_real_plugin -- --ignored --nocapture`
//!
//! 覆盖链路：`Host::load`（包路径解析＋验证激活）→ `activate`（`canProcessSampleSize`
//! 在 nih-plug 布局上命中第 5 槽、0＝接受）→ `process`（音符事件投递＋立体声输出）
//! → 预制切换（`Preset` host 参数，IntParam 0..=119；GS1 不走 VST3 program list）。
//!
//! 单测单实例：GS1 进程内有单实例守卫（第二实例 `initialize` 失败），故本文件
//! 只有一个测试函数，全部验证顺序跑完。

use yeban_plugin_host::{Host, MidiEvent, MidiEventKind, PluginInstance};

/// GS1 bundle 路径：环境变量优先，未设时用本机 synth dist 默认路径。
fn gs1_path() -> String {
    match std::env::var("GS1_VST3_PATH") {
        Ok(path) if !path.is_empty() => path,
        _ => "/home/hatch/workspace/synth/.work/dist/GS1.vst3".to_owned(),
    }
}

fn note_on(offset: u32, pitch: u8, note_id: i32) -> MidiEvent {
    MidiEvent {
        sample_offset: offset,
        kind: MidiEventKind::NoteOn {
            channel: 0,
            pitch,
            velocity: 1.0,
            note_id,
        },
    }
}

fn note_off(offset: u32, pitch: u8, note_id: i32) -> MidiEvent {
    MidiEvent {
        sample_offset: offset,
        kind: MidiEventKind::NoteOff {
            channel: 0,
            pitch,
            velocity: 0.0,
            note_id,
        },
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

/// 自相关估 F0（窗内去均值，滞后取 100–1000 Hz 对应区间的归一化自相关极大值）。
/// 过零率法在 GS1 这类复音色（失谐／滤波／低频分量）上不可靠：预制 0 的
/// 0.2–0.8 秒窗过零率只得 78.3 Hz（窗峰值 0.046，已衰减到尾音），故改自相关法。
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
    // 滞后区间：100–1000 Hz。
    let min_lag = (sample_rate / 1000.0) as usize;
    let max_lag = (sample_rate / 100.0) as usize;
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

/// 两段采样逐点差超 1e-4 的占比（预制是否真切换的判据）。
fn diff_fraction(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "比较的两段采样长度须一致");
    let different = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| (*x - *y).abs() > 1e-4)
        .count();
    different as f32 / a.len() as f32
}

/// 渲染一个音符：块首开音、1.5 秒处关音、共 `seconds` 秒；返回左右声道全段。
/// 块长 4096（须 ≤ 激活块上限）；事件按块内偏移投递。
/// GS1 是真立体声合成器（左右声道内容不同），故分别返回两声道，不作 L＝R 断言。
/// `bend` 为 `Some(v)` 时块首同送弯音事件（归一化值，0.5＝中心）。
fn render_note(
    instance: &mut Box<dyn PluginInstance>,
    note: u8,
    seconds: f64,
    sample_rate: f64,
    block: usize,
    bend: Option<f64>,
) -> (Vec<f32>, Vec<f32>) {
    let total = (seconds * sample_rate) as usize;
    let off_frame = (1.5 * sample_rate) as usize;
    let mut out_l_all = Vec::with_capacity(total);
    let mut out_r_all = Vec::with_capacity(total);
    let mut done = 0_usize;
    while done < total {
        let frames = block.min(total - done);
        let mut events = Vec::new();
        if done == 0 {
            events.push(note_on(0, note, 0));
            if let Some(value) = bend {
                events.push(MidiEvent {
                    sample_offset: 0,
                    kind: MidiEventKind::PitchBend { channel: 0, value },
                });
            }
        }
        if done <= off_frame && off_frame < done + frames {
            events.push(note_off((off_frame - done) as u32, note, 0));
        }
        let mut out_l = vec![0.0_f32; frames];
        let mut out_r = vec![0.0_f32; frames];
        instance
            .process(&events, frames, &mut out_l, &mut out_r)
            .expect("渲染块");
        out_l_all.extend_from_slice(&out_l);
        out_r_all.extend_from_slice(&out_r);
        done += frames;
    }
    (out_l_all, out_r_all)
}

#[test]
#[ignore = "依赖本机外的真 GS1 bundle（见文件头跑法），不进 CI"]
fn gs1_loads_renders_and_switches_presets() {
    const SAMPLE_RATE: f64 = 48_000.0;
    const SECONDS: f64 = 2.0;
    let path = gs1_path();
    assert!(
        std::path::Path::new(&path).exists(),
        "GS1 bundle 不在 {path}（设 GS1_VST3_PATH 指向它）"
    );

    // 1. 装载：包路径解析＋验证激活（失败即报 SampleSizeUnsupported 等）。
    // `Host::load` 只接受乐器类：能加载成功本身即证明 kind＝Instrument
    // （非乐器报 `NotAnInstrument`）。
    let mut instance = Host::load(&path, None).expect("GS1 应能加载");
    eprintln!("类名: {}", instance.name());

    // 2. 参数表：找 `Preset` 选择器（IntParam 0..=119），记其 id 与步数。
    let params = instance.parameters().expect("参数枚举");
    eprintln!("参数总数: {}", params.len());
    let preset = params
        .iter()
        .find(|param| param.title == "Preset")
        .expect("GS1 应有 `Preset` 选择器参数");
    eprintln!(
        "Preset 参数: id={}，step_count={}，缺省归一化={}",
        preset.id, preset.step_count, preset.default_normalized
    );
    assert!(preset.step_count >= 1, "Preset 应为离散参数");
    let preset_id = preset.id;
    let preset_steps = preset.step_count as f64;

    // 3. 激活＋预制 0、A4（69）渲染 2.0 秒。
    instance.activate(SAMPLE_RATE, 4096).expect("激活");
    let (base, base_r) = render_note(&mut instance, 69, SECONDS, SAMPLE_RATE, 4096, None);
    let base_peak = peak(&base).max(peak(&base_r));
    let base_rms = rms(&base);
    let (base_f0, base_window_peak) = estimate_f0(&base, SAMPLE_RATE, 0.02, 0.3);
    eprintln!(
        "预制 0／A4：时长 {:.4} 秒，峰值 {:.6}（左 {:.6}／右 {:.6}），RMS {:.6}，F0 估计 {:.1} Hz（窗峰值 {:.6}）",
        base.len() as f64 / SAMPLE_RATE,
        base_peak,
        peak(&base),
        peak(&base_r),
        base_rms,
        base_f0,
        base_window_peak
    );
    assert!(base_peak > 0.01, "渲染须非静音（峰值 {base_peak}）");
    assert!(
        (300.0..600.0).contains(&base_f0),
        "A4 的 F0 估计应在 300–600 Hz，实为 {base_f0:.1} Hz"
    );

    // 4. 同预制换高音区 A5（81）：F0 应约为 A4 的 2 倍。
    instance.deactivate();
    instance.activate(SAMPLE_RATE, 4096).expect("重激活");
    let (high, high_r) = render_note(&mut instance, 81, SECONDS, SAMPLE_RATE, 4096, None);
    let high_peak = peak(&high).max(peak(&high_r));
    let (high_f0, _) = estimate_f0(&high, SAMPLE_RATE, 0.02, 0.3);
    eprintln!(
        "预制 0／A5：峰值 {:.6}，F0 估计 {:.1} Hz（A4 的 {:.2} 倍）",
        high_peak,
        high_f0,
        high_f0 / base_f0
    );
    assert!(high_peak > 0.01, "高音区渲染须非静音");
    assert!(
        (1.8..2.2).contains(&(high_f0 / base_f0)),
        "A5／A4 的 F0 比应 ≈2，实为 {:.2}",
        high_f0 / base_f0
    );

    // 5. 预制切换：依次试 1、5、10，取首个与预制 0 输出明显不同的。
    // 直设必须在停用状态下做：nih-plug 的 `setParamNormalized` 在处理中
    // （`is_processing`）时直接返回 Ok 而不写值（参数变化须走 process 的
    // `input_parameter_changes`）；激活态直设会静默丢失，读回仍为旧值。
    let mut switched = None;
    for candidate in [1_i32, 5, 10] {
        instance.deactivate();
        let normalized = candidate as f64 / preset_steps;
        instance
            .set_parameter(preset_id, normalized)
            .expect("置 Preset 参数");
        let read_back = instance
            .parameter_value(preset_id)
            .expect("Preset 参数应可读");
        assert!(
            (read_back - normalized).abs() < 1e-6,
            "Preset 回读应 ≈ {normalized}，实为 {read_back}"
        );
        instance.activate(SAMPLE_RATE, 4096).expect("重激活");
        let (rendered, rendered_r) =
            render_note(&mut instance, 69, SECONDS, SAMPLE_RATE, 4096, None);
        let rendered_peak = peak(&rendered).max(peak(&rendered_r));
        let diff = diff_fraction(&base, &rendered);
        eprintln!(
            "预制 {candidate}／A4：峰值 {:.6}，RMS {:.6}，与预制 0 逐点差异占比 {:.3}",
            rendered_peak,
            rms(&rendered),
            diff
        );
        if diff > 0.1 {
            switched = Some((candidate, diff));
            break;
        }
    }
    let (switched_preset, diff) = switched.expect("至少一个候选预制应与预制 0 输出不同");
    eprintln!("预制切换证实：预制 {switched_preset} 与预制 0 差异占比 {diff:.3}");

    // 6. 弯音经参数变化通道送达：预制重置 0 后，控制渲染与弯音 1.0 渲染对比。
    // 直设必须在停用状态下做（与 §5 同款原因：激活态直设静默丢失）。
    instance.deactivate();
    instance.set_parameter(preset_id, 0.0).expect("预制重置 0");
    instance.activate(SAMPLE_RATE, 4096).expect("重激活");
    let (control, _) = render_note(&mut instance, 69, SECONDS, SAMPLE_RATE, 4096, None);
    let (control_f0, _) = estimate_f0(&control, SAMPLE_RATE, 0.02, 0.3);
    let (bent, bent_r) = render_note(&mut instance, 69, SECONDS, SAMPLE_RATE, 4096, Some(1.0));
    let bent_peak = peak(&bent).max(peak(&bent_r));
    let (bent_f0, _) = estimate_f0(&bent, SAMPLE_RATE, 0.02, 0.3);
    eprintln!(
        "弯音：控制 F0 {control_f0:.1} Hz，弯音 1.0 F0 {bent_f0:.1} Hz（比值 {:.3}），弯音峰值 {bent_peak:.6}",
        bent_f0 / control_f0
    );
    assert!(bent_peak > 0.01, "弯音渲染须非静音");
    assert!(
        (400.0..480.0).contains(&control_f0),
        "控制渲染 F0 应 ≈440 Hz，实为 {control_f0:.1} Hz"
    );
    assert!(
        (1.08..1.17).contains(&(bent_f0 / control_f0)),
        "弯音 1.0／控制的 F0 比应 ≈1.122（＋2 半音），实为 {:.3}",
        bent_f0 / control_f0
    );
}
