//! 离线验证驱动：take + 粗起点 TSV → 精修 → timemap → 单实例 warped WAV。
//!
//! 用法：
//! ```text
//! cargo run -p yeban-dsp --release --example vocal_align -- \
//!   --take lead_take0.wav --notes notes.tsv \
//!   --out-wav warped.wav --report report.tsv \
//!   --out-end 4.05 --tail-in-end 4.30 [--config preset] [--align-drop N]
//!   [--residual-mix [--residual-gain 1.0] [--residual-cutoff 1000]]
//! ```
//!
//! `--residual-mix`（P0-2B 证伪实验）：从原声 take 提非周期高频残差，
//! 按同一 timemap 弯曲后混入拉伸输出，只在锚定 VOP 窗口内；默认关闭，
//! 关闭时输出与不加开关逐位相同。收据行写入报告 TSV。
//!
//! 输入 TSV（无表头，制表符分隔）：
//! `idx score_vop_s score_end_s coarse_s prev_bound_s next_bound_s`
//!
//! * `score_vop_s`：目标 VOP（行相对秒；全曲模式为曲相对秒）。
//! * `score_end_s`：乐谱行尾音结束（仅行尾音用于门检跨度；其余行可填 0）。
//! * `coarse_s`：回执粗起点（take 内秒）。
//! * `prev_bound_s` / `next_bound_s`：前后音节边界（take 内秒）。
//!
//! 输出 TSV（制表符分隔）：先是 `#` 注释行（门检 verdict、钳制事件、
//! 渲染报告、未锚定音节），后是每音节一行：
//! `idx anchored pre_s vop_s off_s confidence f0_median_hz hnr_db
//! target_vop_s core_required_rate core_clamped_rate`。
//!
//! 本驱动只做离线验证，不进入任何实时路径。

use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;
use yeban_dsp::vocal_refine::{RefinedNote, breath_spans, detect_adlib, refine_note};
use yeban_dsp::vocal_residual_mix::{
    ResidualMixConfig, ResidualMixReceipt, extract_aperiodic_high, mix_residual, warp_residual,
};
use yeban_dsp::vocal_timemap::{
    StretchConfig, SyllableIn, SyllableTarget, TimemapOptions, build_timemap, gate_verdict,
    render_continuous_timemap,
};

struct Args {
    take: String,
    notes: String,
    out_wav: String,
    report: String,
    out_end: f64,
    tail_in_end: f64,
    config: StretchConfig,
    align_drop: Option<usize>,
    adlib_start: Option<f64>,
    adlib_end: Option<f64>,
    residual_mix: bool,
    residual_gain: Option<f32>,
    residual_cutoff: Option<f32>,
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut take = None;
    let mut notes = None;
    let mut out_wav = None;
    let mut report = None;
    let mut out_end = None;
    let mut tail_in_end = None;
    let mut config = StretchConfig::Custom2400_480;
    let mut align_drop = None;
    let mut adlib_start = None;
    let mut adlib_end = None;
    let mut residual_mix = false;
    let mut residual_gain = None;
    let mut residual_cutoff = None;
    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].as_str();
        let value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            argv.get(*i)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "--take" => take = Some(value(&mut i)?),
            "--notes" => notes = Some(value(&mut i)?),
            "--out-wav" => out_wav = Some(value(&mut i)?),
            "--report" => report = Some(value(&mut i)?),
            "--out-end" => {
                out_end = Some(value(&mut i)?.parse::<f64>().map_err(|e| e.to_string())?)
            }
            "--tail-in-end" => {
                tail_in_end = Some(value(&mut i)?.parse::<f64>().map_err(|e| e.to_string())?);
            }
            "--config" => {
                config = match value(&mut i)?.as_str() {
                    "custom" => StretchConfig::Custom2400_480,
                    "preset" => StretchConfig::PresetDefault,
                    other => return Err(format!("unknown config {other}")),
                };
            }
            "--align-drop" => {
                align_drop = Some(value(&mut i)?.parse::<usize>().map_err(|e| e.to_string())?);
            }
            "--adlib-start" => {
                adlib_start = Some(value(&mut i)?.parse::<f64>().map_err(|e| e.to_string())?);
            }
            "--adlib-end" => {
                adlib_end = Some(value(&mut i)?.parse::<f64>().map_err(|e| e.to_string())?);
            }
            "--residual-mix" => residual_mix = true,
            "--residual-gain" => {
                residual_gain = Some(value(&mut i)?.parse::<f32>().map_err(|e| e.to_string())?);
            }
            "--residual-cutoff" => {
                residual_cutoff = Some(value(&mut i)?.parse::<f32>().map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown flag {other}")),
        }
        i += 1;
    }
    // adlib 模式只须 --take + --adlib-start/end，其余参数可省。
    Ok(Args {
        take: take.ok_or("--take required")?,
        notes: notes.unwrap_or_default(),
        out_wav: out_wav.unwrap_or_default(),
        report: report.unwrap_or_default(),
        out_end: out_end.unwrap_or(0.0),
        tail_in_end: tail_in_end.unwrap_or(0.0),
        config,
        align_drop,
        adlib_start,
        adlib_end,
        residual_mix,
        residual_gain,
        residual_cutoff,
    })
}

/// 最小 WAV 读入：PCM16/24/32/float，单声道；非单声道报错。
fn read_wav(path: &str) -> Result<(Vec<f32>, u32), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_owned());
    }
    let mut pos = 12;
    let mut format = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut pcm: Vec<f32> = Vec::new();
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if body + size > data.len() {
            break;
        }
        match id {
            b"fmt " => {
                format = u16::from_le_bytes(data[body..body + 2].try_into().unwrap());
                channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(data[body + 4..body + 8].try_into().unwrap());
                bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().unwrap());
            }
            b"data" => {
                let bytes = &data[body..body + size];
                match (format, bits) {
                    (1, 16) => {
                        for chunk in bytes.as_chunks::<2>().0 {
                            let v = i16::from_le_bytes(*chunk);
                            pcm.push(f32::from(v) / 32768.0);
                        }
                    }
                    (1, 24) => {
                        for chunk in bytes.as_chunks::<3>().0 {
                            let v = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], 0]) >> 8;
                            pcm.push(v as f32 / 8_388_608.0);
                        }
                    }
                    (1, 32) => {
                        for chunk in bytes.as_chunks::<4>().0 {
                            let v = i32::from_le_bytes(*chunk);
                            pcm.push(v as f32 / 2_147_483_648.0);
                        }
                    }
                    (3, 32) => {
                        for chunk in bytes.as_chunks::<4>().0 {
                            pcm.push(f32::from_le_bytes(*chunk));
                        }
                    }
                    _ => return Err(format!("unsupported wav format {format} bits {bits}")),
                }
            }
            _ => {}
        }
        pos = body + size + (size % 2);
    }
    if channels != 1 {
        return Err(format!("only mono supported, got {channels} channels"));
    }
    if pcm.is_empty() {
        return Err("no audio data".to_owned());
    }
    Ok((pcm, sample_rate))
}

/// PCM16 单声道写出。
fn write_wav(path: &str, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let mut out = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let data_bytes = samples.len() * 2;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    out.write_all(&header).map_err(|e| e.to_string())?;
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * 32767.0) as i16;
        out.write_all(&value.to_le_bytes())
            .map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(())
}

struct NoteRow {
    idx: usize,
    score_vop: f64,
    score_end: f64,
    coarse: f64,
    prev_bound: f64,
    next_bound: f64,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("vocal_align: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = run(&args) {
        eprintln!("vocal_align: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run(args: &Args) -> Result<(), String> {
    let (take, sample_rate) = read_wav(&args.take)?;
    if let (Some(start), Some(end)) = (args.adlib_start, args.adlib_end) {
        // adlib 检出模式：间奏窗内持续 >200 ms 且自相关 >0.75 的有调段。
        let spans = detect_adlib(&take, sample_rate, &[(start, end)]);
        for span in &spans {
            println!(
                "adlib {:.4} {:.4} f0={:.1} peak={:.2} dur={:.3}",
                span.start_s,
                span.end_s,
                span.f0_median_hz,
                span.autocorr_peak,
                span.end_s - span.start_s
            );
        }
        eprintln!("vocal_align: adlib_spans={}", spans.len());
        return Ok(());
    }
    let file = File::open(&args.notes).map_err(|e| e.to_string())?;
    let mut rows: Vec<NoteRow> = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 6 {
            return Err(format!("bad notes line (need 6 fields): {line}"));
        }
        let parse = |field: &str| field.parse::<f64>().map_err(|e| e.to_string());
        rows.push(NoteRow {
            idx: fields[0].parse::<usize>().map_err(|e| e.to_string())?,
            score_vop: parse(fields[1])?,
            score_end: parse(fields[2])?,
            coarse: parse(fields[3])?,
            prev_bound: parse(fields[4])?,
            next_bound: parse(fields[5])?,
        });
    }
    if rows.is_empty() {
        return Err("no notes".to_owned());
    }
    let mut refined: Vec<(NoteRow, RefinedNote)> = Vec::new();
    for row in rows {
        let note = refine_note(
            &take,
            sample_rate,
            row.coarse,
            row.prev_bound,
            row.next_bound,
        );
        refined.push((row, note));
    }
    // 同事件碰撞：两锚定 VOP 相距 < 10 ms（一个 hop）必是同一
    // 起音事件被两音节认领（行 5 注 41/42 实测相距 3.3 ms），留
    // |VOP−coarse| 小者（粗起点近者为真主），另一者弃锚。21 ms 级
    // 的爬坡尾帧不是同事件（行 13 缝/隙），不在此列。
    let mut collision_dropped: Vec<usize> = Vec::new();
    let mut last_anchored_pos: Option<usize> = None;
    for pos in 0..refined.len() {
        if !refined[pos].1.anchored {
            continue;
        }
        if let Some(last) = last_anchored_pos {
            let last_idx = refined[last].0.idx;
            let last_vop = refined[last].1.vop_s;
            let last_coarse = refined[last].0.coarse;
            let cur_idx = refined[pos].0.idx;
            let cur_vop = refined[pos].1.vop_s;
            let cur_coarse = refined[pos].0.coarse;
            if (cur_vop - last_vop).abs() < 0.010 {
                let last_err = (last_vop - last_coarse).abs();
                let cur_err = (cur_vop - cur_coarse).abs();
                if cur_err > last_err {
                    refined[pos].1.anchored = false;
                    collision_dropped.push(cur_idx);
                } else {
                    refined[last].1.anchored = false;
                    collision_dropped.push(last_idx);
                    last_anchored_pos = Some(pos);
                }
                continue;
            }
        }
        last_anchored_pos = Some(pos);
    }
    // 第二遍：只重精修首遍扩展窗匹配的音节（|VOP−coarse| > 80 ms；
    // 首窗匹配必在 ±80 ms 内，否则首窗早已命中，故该判据等价于
    // “扩展窗匹配”)，搜索下限取前方最近保留锚定 VOP + 65 ms
    // （两锚定起音不可能更近；未锚定行不重置该界）。严格界无果时
    // 以 TSV 宽松界重试（前一 VOP 本身可能错认）；两遍皆无果则弃锚
    // （首遍扩展 VOP 未被界内证据确认，不可信）。
    let mut bound_relaxed: Vec<usize> = Vec::new();
    let mut pass2_dropped: Vec<usize> = Vec::new();
    let mut prev_kept_vop: Option<f64> = None;
    for (row, note) in refined.iter_mut() {
        let expanded = note.anchored && (note.vop_s - row.coarse).abs() > 0.080;
        if expanded {
            let loose = row.prev_bound;
            let strict = prev_kept_vop
                .map(|v| (v + 0.065).max(loose))
                .unwrap_or(loose);
            let mut new_note = refine_note(&take, sample_rate, row.coarse, strict, row.next_bound);
            let mut relaxed = false;
            if !new_note.anchored && strict > loose {
                let retry = refine_note(&take, sample_rate, row.coarse, loose, row.next_bound);
                if retry.anchored {
                    new_note = retry;
                    relaxed = true;
                }
            }
            if new_note.anchored {
                if relaxed {
                    bound_relaxed.push(row.idx);
                }
                *note = new_note;
            } else {
                note.anchored = false;
                pass2_dropped.push(row.idx);
            }
        }
        if note.anchored {
            prev_kept_vop = Some(note.vop_s);
        }
    }
    let anchored_refined: Vec<(&NoteRow, &RefinedNote)> = refined
        .iter()
        .filter(|(_, note)| note.anchored)
        .map(|(row, note)| (row, note))
        .collect();
    if anchored_refined.is_empty() {
        return Err("no anchored notes".to_owned());
    }
    // 平铺：非末音节 off 不越过下一锚定音节 pre − 10 ms；末音节 off 不
    // 越过行尾输入终点。连唱浊辅音延续时 off 常越界，不钳制则映射回跳。
    // 平铺后仍容不下 65 ms（35 ms 锁窗 + 30 ms 收音尾）的音节直接弃锚：
    // 喂给构建器会产生零长核（门检里按 1.0 计入，虚增通过率），不如让其
    // 音频以 1.0 ride 在间隙里、下一锚定音节的锁窗仍钉住其 VOP。
    // 单遍正向处理：被弃音节的 pre 仍作前一音节的钳制上限（更紧但安全）。
    let anchored_count = anchored_refined.len();
    let mut dropped: Vec<usize> = Vec::new();
    let mut syllables: Vec<SyllableIn> = Vec::new();
    let mut anchored: Vec<(&NoteRow, &RefinedNote)> = Vec::new();
    for (pos, (row, note)) in anchored_refined.iter().enumerate() {
        let off_limit = if pos + 1 < anchored_count {
            anchored_refined[pos + 1].1.pre_s - 0.010
        } else {
            args.tail_in_end
        };
        let off_c = note.off_s.min(off_limit);
        if off_c - note.vop_s < 0.065 {
            dropped.push(row.idx);
            continue;
        }
        syllables.push(SyllableIn {
            pre_s: note.pre_s,
            vop_s: note.vop_s,
            off_s: off_c,
        });
        anchored.push((row, note));
    }
    if anchored.is_empty() {
        return Err("no anchored notes after tiling".to_owned());
    }
    let targets: Vec<SyllableTarget> = anchored
        .iter()
        .map(|(row, _)| SyllableTarget {
            vop_s: row.score_vop,
        })
        .collect();
    let build = build_timemap(
        &take,
        sample_rate,
        &syllables,
        &targets,
        args.out_end,
        args.tail_in_end,
        TimemapOptions::default(),
    );
    let core_rates: Vec<f64> = build
        .intervals
        .iter()
        .filter(|interval| interval.is_voiced_core)
        .map(|interval| interval.rate)
        .collect();
    let first = anchored.first().expect("non-empty");
    let last_row = anchored.last().expect("non-empty");
    let last_syllable = syllables.last().expect("non-empty");
    // 行比来源：首末音节均（平铺后仍）锚定时用锚定跨度（精确）；否则
    // 用段跨度（首 coarse → 末 next_bound，含尾部静音/换气，近似并标记）。
    let first_refined = &refined.first().expect("non-empty").1;
    let last_refined = &refined.last().expect("non-empty").1;
    let first_final =
        first_refined.anchored && !dropped.contains(&refined.first().expect("non-empty").0.idx);
    let last_final =
        last_refined.anchored && !dropped.contains(&refined.last().expect("non-empty").0.idx);
    let (line_ratio, ratio_source) = if first_final && last_final {
        let sung_span = (last_syllable.off_s - first.1.vop_s).max(1e-9);
        let score_span = (last_row.0.score_end - first.0.score_vop).max(1e-9);
        (score_span / sung_span, "anchored")
    } else {
        let first_row = &refined.first().expect("non-empty").0;
        let last_row_all = &refined.last().expect("non-empty").0;
        // 段尾：末行已锚定时用其实测 off（段界常含尾部静音/间奏，用
        // next_bound 会把行比拖垮：行 13 得 0.681、行 14 得 0.231）；
        // 末行未锚定时才退回段界。
        let sung_end = if last_final {
            last_syllable.off_s
        } else {
            last_row_all.next_bound
        };
        let sung_span = (sung_end - first_row.coarse).max(1e-9);
        let score_span = (last_row_all.score_end - first_row.score_vop).max(1e-9);
        (score_span / sung_span, "segment")
    };
    let verdict = gate_verdict(line_ratio, &core_rates);
    // 渲染钳制本身即门检失败（Q13 钳制界 [0.60, 1.65] 内的需求才可忠实
    // 渲染；超限核的空缺间隙只能喂静音，输出已偏离目标网格，必须重抽）。
    let gate_pass_reported = verdict.pass && build.clamps.is_empty();
    let gaps: Vec<(f64, f64)> = syllables
        .windows(2)
        .map(|pair| (pair[0].off_s, pair[1].pre_s))
        .collect();
    let breaths = breath_spans(&take, sample_rate, &gaps);
    let target_len = (args.out_end * sample_rate as f64).round() as usize;
    let (rendered, render_report) = render_continuous_timemap(
        &take,
        sample_rate,
        &build.map,
        target_len,
        args.config,
        args.align_drop,
    );
    // P0-2B：残差混合（默认关闭）。VOP 锚点取锚定音节的目标 VOP
    // （输出秒）；残差从原声 take 提取、按同一 timemap 弯曲。
    let (final_audio, residual_receipt): (Vec<f32>, Option<ResidualMixReceipt>) =
        if args.residual_mix {
            let config = ResidualMixConfig {
                enabled: true,
                sample_rate,
                mix_gain: args.residual_gain.unwrap_or(1.0),
                hf_cutoff_hz: args.residual_cutoff.unwrap_or(1000.0),
                ..ResidualMixConfig::default()
            };
            let residual = extract_aperiodic_high(&take, &config).map_err(|e| e.to_string())?;
            let warped = warp_residual(&residual, &build.map, rendered.len(), &config)
                .map_err(|e| e.to_string())?;
            let vop_targets: Vec<f64> = anchored.iter().map(|(row, _)| row.score_vop).collect();
            let (mixed, receipt) = mix_residual(&rendered, &warped, &vop_targets, &config)
                .map_err(|e| e.to_string())?;
            (mixed, Some(receipt))
        } else {
            (rendered, None)
        };
    write_wav(&args.out_wav, &final_audio, sample_rate)?;
    let mut report = BufWriter::new(File::create(&args.report).map_err(|e| e.to_string())?);
    writeln!(report, "# gate_pass={gate_pass_reported}").map_err(|e| e.to_string())?;
    writeln!(report, "# gate_rate_pass={}", verdict.pass).map_err(|e| e.to_string())?;
    writeln!(report, "# gate_ratio_source={ratio_source}").map_err(|e| e.to_string())?;
    writeln!(report, "# line_ratio={line_ratio:.4}").map_err(|e| e.to_string())?;
    writeln!(
        report,
        "# in_range_fraction={:.4}",
        verdict.in_range_fraction
    )
    .map_err(|e| e.to_string())?;
    writeln!(report, "# out_of_hard_count={}", verdict.out_of_hard_count)
        .map_err(|e| e.to_string())?;
    for failure in &verdict.failures {
        writeln!(report, "# gate_failure={failure}").map_err(|e| e.to_string())?;
    }
    if !build.clamps.is_empty() {
        writeln!(
            report,
            "# gate_failure=render clamps present, retry required"
        )
        .map_err(|e| e.to_string())?;
    }
    for idx in &dropped {
        writeln!(
            report,
            "# tiling_dropped={idx} off below vop+65ms after tiling"
        )
        .map_err(|e| e.to_string())?;
    }
    for idx in &bound_relaxed {
        writeln!(
            report,
            "# bound_relaxed={idx} strict prev-vop bound yielded nothing, TSV bound retry anchored"
        )
        .map_err(|e| e.to_string())?;
    }
    for idx in &collision_dropped {
        writeln!(
            report,
            "# collision_dropped={idx} VOP within 10ms of another note, worse coarse fit"
        )
        .map_err(|e| e.to_string())?;
    }
    for idx in &pass2_dropped {
        writeln!(
            report,
            "# pass2_dropped={idx} expanded VOP not confirmed under prev-vop bound"
        )
        .map_err(|e| e.to_string())?;
    }
    for (out_s, in_s) in &build.map.knots {
        writeln!(report, "# knot {out_s:.4} {in_s:.4}").map_err(|e| e.to_string())?;
    }
    writeln!(report, "# build_clamps={}", build.clamps.len()).map_err(|e| e.to_string())?;
    for clamp in &build.clamps {
        writeln!(
            report,
            "# build_clamp out_s={:.4} requested={:.3} clamped={:.3}",
            clamp.out_s, clamp.requested_rate, clamp.clamped_rate
        )
        .map_err(|e| e.to_string())?;
    }
    for warning in &build.warnings {
        writeln!(report, "# build_warning={warning}").map_err(|e| e.to_string())?;
    }
    writeln!(
        report,
        "# render_step_clamps={} out_len={}",
        render_report.step_clamps, render_report.out_len
    )
    .map_err(|e| e.to_string())?;
    for (cursor, requested, clamped) in &render_report.step_clamp_details {
        writeln!(
            report,
            "# render_step_clamp cursor={cursor} requested={requested} clamped={clamped}"
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(receipt) = &residual_receipt {
        writeln!(
            report,
            "# residual_mix=1 gain={:.3} cutoff_hz={:.0} peak_in={:.4} peak_out={:.4} windows={}",
            args.residual_gain.unwrap_or(1.0),
            args.residual_cutoff.unwrap_or(1000.0),
            receipt.peak_in,
            receipt.peak_out,
            receipt.windows
        )
        .map_err(|e| e.to_string())?;
        for window in &receipt.per_window {
            writeln!(
                report,
                "# residual_window target={:.4} peak_residual={:.4} flux_offset={} misaligned={}",
                window.target_vop_s,
                window.peak_residual,
                window
                    .flux_peak_offset_samples
                    .map(|o| o.to_string())
                    .unwrap_or_else(|| "none".to_owned()),
                window.misaligned
            )
            .map_err(|e| e.to_string())?;
        }
    } else {
        writeln!(report, "# residual_mix=0").map_err(|e| e.to_string())?;
    }
    writeln!(report, "# breath_spans={}", breaths.len()).map_err(|e| e.to_string())?;
    for span in &breaths {
        writeln!(
            report,
            "# breath_span {:.4} {:.4} {:?} {:.1}dB",
            span.start_s, span.end_s, span.kind, span.rms_db
        )
        .map_err(|e| e.to_string())?;
    }
    let mut core_iter = build
        .intervals
        .iter()
        .filter(|interval| interval.is_voiced_core);
    let mut anchored_pos = 0usize;
    for (row, note) in &refined {
        let final_anchored = note.anchored && !dropped.contains(&row.idx);
        let (required, clamped) = if final_anchored {
            let interval = core_iter.next();
            (
                interval.map(|i| i.rate).unwrap_or(1.0),
                interval.map(|i| i.rate_clamped).unwrap_or(1.0),
            )
        } else {
            (0.0, 0.0)
        };
        // 报告列出的 off 是平铺钳制后的实际使用值（与精修原值可能不同）；
        // anchored 列是平铺后最终状态（被弃音节记 0）。
        let used_off = if final_anchored {
            let off = syllables[anchored_pos].off_s;
            anchored_pos += 1;
            off
        } else {
            note.off_s
        };
        writeln!(
            report,
            "{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.3}\t{:.1}\t{:.1}\t{:.4}\t{:.3}\t{:.3}",
            row.idx,
            if final_anchored { 1 } else { 0 },
            note.pre_s,
            note.vop_s,
            used_off,
            note.confidence,
            note.f0_median_hz,
            note.hnr_db,
            row.score_vop,
            required,
            clamped
        )
        .map_err(|e| e.to_string())?;
    }
    report.flush().map_err(|e| e.to_string())?;
    eprintln!(
        "vocal_align: gate_pass={gate_pass_reported} rate_pass={} ratio_source={ratio_source} line_ratio={line_ratio:.4} clamps={} step_clamps={} out={}",
        verdict.pass,
        build.clamps.len(),
        render_report.step_clamps,
        args.out_wav
    );
    Ok(())
}
