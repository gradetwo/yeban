//! B8 端到端判据：**轨级 master tune 真的改变渲染音高**。
//!
//! ## 来历（B8，报告第四份「本轮最想要」）
//!
//! `setTrackTune`（`TrackV3::tune_offset_cents`，±100 音分）只在**轨级**生效：
//! `yeban_render_sfz_wav` 渲染单个音符、没有音轨概念，观测不到它。本判据走
//! **工程**通路：`filled_project`（Lead 轨音高 60 音符）→ `yeban_edit_notes`
//!（`setTrackSampler` 绑采样器 ＋ `setTrackTune` 设偏移）→ `yeban_merge_proposal`
//! → `yeban_render_master` → 测音高。
//!
//! ## 夹具自包含
//!
//! 440 Hz 正弦 WAV 由本文件内的 `sine_wav` 生成（16 位单声道，48 kHz，2 s），
//! SFZ 为 `<region> sample=sine.wav key=60 pitch_keycenter=60`（音高比 = 1 的
//! 不动点）。不依赖外部文件或 ffmpeg。量测形状照 B1
//!（`flac_mono_pitch_e2e.rs` 的 `measure_freq_cents` 与 `wav_read`）。
//!
//! ## 判定量的口径
//!
//! - **量**：基频（Hz），由稳态段内**上升零交叉**（线性插值定分数时刻）的平均
//!   周期换算；报告量为相对 440.00 Hz 的音分偏差 `1200 * log2(f / 440)`。
//! - 窗口 0.02 s..0.18 s：Lead 轨音高 60 音符只在 0..0.25 s 发声（480 tick @
//!   120 BPM / 960 PPQ），Bass 的 kick 在 1.0 s 才进，窗内只有这一音。
//! - 门限 **10 音分**（B1 同口径；本路径实测误差应 < 1 音分）。
//! - 并断言 `sources[0].sampler.notesUsingFileSample == 1`，防止把正弦回退
//!   当样本（回退也是 440 Hz，单看音高分辨不出）。
//!
//! ## 覆盖的设定值
//!
//! | 设定（音分） | 含义 |
//! | --- | --- |
//! | 0 | 对照：缺省 0 不改变渲染 |
//! | −100 | 下界；ixox 长笛约 −100 音分偏差（B1） |
//! | −31.766068… | 432 Hz 对 440 Hz（`1200·log2(432/440)`） |
//! | +100 | 上界 |

use std::path::PathBuf;

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;

// ─── 量测（照 B1） ─────────────────────────────────────────────────────────

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
        "交叉点太少（{}），信号可能不是正弦",
        crossings.len()
    );
    let span = crossings[crossings.len() - 1] - crossings[0];
    let freq = (crossings.len() - 1) as f64 / span * f64::from(rate);
    (freq, 1200.0 * (freq / 440.0).log2())
}

// ─── 夹具 ──────────────────────────────────────────────────────────────────

/// 生成 16 位单声道 440 Hz 正弦 WAV 字节（与 B1 的 `sine_wav` 同值）。
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

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-track-tune-e2e-{tag}-{}.yeban",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

// ─── 渲染脚手架（照 B1 与 render_master.rs 的形状） ────────────────────────

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-track-tune-e2e-{}.yeban", std::process::id())),
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

// ─── 判据本体 ──────────────────────────────────────────────────────────────

/// `filled_project` 里 Lead 轨的身份（按名找，不抄样本内部的 fixture id）。
fn lead_track_id() -> String {
    filled_project()
        .tracks
        .values()
        .find(|track| track.name == "Lead")
        .expect("夹具必须有 Lead 轨")
        .id
        .to_canonical_string()
}

#[test]
fn track_tune_shifts_the_rendered_pitch_by_the_set_cents() {
    let dir = test_dir("render");
    let wav_path = dir.join("sine.wav");
    std::fs::write(&wav_path, sine_wav(48_000, 96_000)).expect("写正弦夹具");
    let sfz_path = dir.join("tune.sfz");
    std::fs::write(
        &sfz_path,
        "<region> sample=sine.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 SFZ");
    let lead = lead_track_id();
    // 432 Hz 对 440 Hz 的音分差（与设定值用同一表达式，免得手抄漂移）。
    let cents_432 = 1200.0 * (432.0f64 / 440.0).log2();

    for (tune_cents, label) in [
        (0.0, "对照 0"),
        (-100.0, "下界 −100"),
        (cents_432, "432 Hz"),
        (100.0, "上界 +100"),
    ] {
        let mut dispatcher = dispatcher_with_project();
        // 绑采样器 ＋ 设调律偏移（同一次提案），再合入。
        let edited = call_tool(
            &mut dispatcher,
            "yeban_edit_notes",
            serde_json::json!({
                "trackId": lead,
                "ops": [
                    {"kind": "setTrackSampler", "trackId": lead, "sampler": sfz_path.display().to_string()},
                    {"kind": "setTrackTune", "trackId": lead, "tuneCents": tune_cents},
                ],
            }),
        );
        assert_eq!(edited["status"], "success", "{label} 编辑: {edited}");
        let proposal = edited["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("proposalId")
            .to_owned();
        let merged = call_tool(
            &mut dispatcher,
            "yeban_merge_proposal",
            serde_json::json!({"proposalId": proposal, "commitMessage": "B8 track tune e2e"}),
        );
        assert_eq!(
            merged["data"]["merged"], true,
            "{label} 提案必须真的被施加: {merged}"
        );

        let target = dir.join(format!("tune-{tune_cents}.wav"));
        let rendered = call_tool(
            &mut dispatcher,
            "yeban_render_master",
            serde_json::json!({
                "format": "wav",
                "sampleRate": 48000,
                "path": target.display().to_string(),
            }),
        );
        assert_eq!(rendered["status"], "success", "{label} 渲染: {rendered}");
        assert_eq!(
            rendered["data"]["sources"][0]["kind"], "sfz-sampler",
            "{label} Lead 轨必须走采样器: {rendered}"
        );
        assert_eq!(
            rendered["data"]["sources"][0]["sampler"]["notesUsingFileSample"], 1,
            "{label} 必须真用文件样本（⛔ 不是正弦回退）: {rendered}"
        );
        let bytes = std::fs::read(&target).expect("读回渲染 WAV");
        let (channels, rate, samples) = wav_read(&bytes);
        assert_eq!((channels, rate), (2, 48_000), "{label} WAV 头");
        let left: Vec<f32> = samples
            .iter()
            .step_by(2)
            .map(|s| *s as f32 / 8_388_608.0)
            .collect();
        let (_, cents) = measure_freq_cents(&left, rate, 0.02, 0.18);
        assert!(
            (cents - tune_cents).abs() <= 10.0,
            "{label} 实测 {cents:+.2} 音分，设定 {tune_cents:+.2} 音分（门限 10）"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
