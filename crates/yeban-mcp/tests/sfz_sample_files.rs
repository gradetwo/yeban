//! **SFZ `sample=<path>` 真读 WAV** 的承重判据（目标 A① 在 MCP 路径上的那一半）。
//!
//! ## 覆盖的判据
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 同一 SFZ、同一音符：`sample=` 指向**真实** WAV ｖｓ 指向**不存在**的路径 ⇒ 两次渲染的 WAV **可区分**（逐样本最大绝对差 ≠ 0）＋ **两侧都非静音** | `a_real_sample_file_changes_the_rendered_bytes_and_both_sides_are_audible` |
//! | b | **常量样本**（全 `0.25`）⇒ 文件侧的**持续段极差 ≈ 0**、内存正弦侧极差明显 | 同上 |
//! | c | 装载响应如实：读到 ⇒ `sampleFilesRead=true` ＋ 区域 `sampleFileState=file(<path> frames=N rate=R)`；读不到 ⇒ `unreadable(...)` 带原因，且**装载仍然成功** | `the_load_response_reports_the_file_it_really_read` |
//! | d | **上界**：单文件超过字节上限 ⇒ **拒绝读取**并记原因（⛔ 不静默截断），装载仍成功 | `an_oversized_sample_file_is_refused_loudly_and_does_not_break_the_load` |
//! | e | 相对路径按 **SFZ 文件所在目录**解析（⛔ 不是进程 cwd） | `the_load_response_reports_the_file_it_really_read`（夹具与 SFZ 同目录，cwd 不是那里） |
//!
//! ## 判定量的口径（先命名，再测）
//!
//! - **样本**：`i24` 整数域（±8388607）里的**峰值**与**持续段极差**（`max − min`）。
//!   "持续段" = 每个音符写出的帧区间**去掉末尾 64 帧淡出**（本渲染器对每个音符做末尾线性
//!   淡出 ⇒ 全段极差必然含那段斜坡，那是**包络**不是样本内容）。
//!   ⚠ 与 `app` 侧读数的差别如实登记：`app` 的 `SfzTrackSource` **没有**这个淡出，
//!   所以它的文件侧极差是 `0.000000`；本侧只在**持续段**上取极差。
//! - **可区分**：两次渲染的 `i24` 样本逐点差的最大绝对值（`> 0` 即不同）。
//!
//! 夹具 WAV 是**手写字节**（常量 `0.25` 的 24-bit 单声道 48 kHz）⇒ ⛔ 不依赖被测写入器。

use std::path::{Path, PathBuf};

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-sfzfile-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// **手写**一份常量值的 24-bit 单声道 WAV（⛔ 不用被测写入器自证）。
///
/// `value` ∈ `[-1, 1]`；写成 `i24` 整数域里的同一个值。
fn write_constant_wav(path: &Path, value: f64, frames: usize, sample_rate: u32) {
    let scaled = (value * ((1_i32 << 23) - 1) as f64).round() as i32;
    let mut data = Vec::with_capacity(frames * 3);
    for _ in 0..frames {
        let bytes = scaled.to_le_bytes();
        data.extend_from_slice(&bytes[..3]);
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 3).to_le_bytes()); // byte rate
    out.extend_from_slice(&3_u16.to_le_bytes()); // block align
    out.extend_from_slice(&24_u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, &out).expect("写常量样本夹具");
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-sfzfile-{}.yeban", std::process::id())),
            filled_project(),
            false,
        )
        .expect("注入规范工程");
    dispatcher
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
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

/// **独立**的极小 RIFF 解析器（i24 交错样本；与 `sfz_tools.rs` 同一口径）。
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

#[test]
fn a_real_sample_file_changes_the_rendered_bytes_and_both_sides_are_audible() {
    let scratch = Scratch::new("real");
    // 常量样本：全 0.25（24-bit 单声道 48 kHz，48,000 帧 = 1 秒）。
    let sample_path = scratch.join("const.wav");
    write_constant_wav(&sample_path, 0.25, 48_000, 48_000);

    // 同一份 SFZ 文本：`sample=` 分别指向**真实文件**与**不存在的文件**。
    let with_file = scratch.join("with-file.sfz");
    std::fs::write(
        &with_file,
        "<region> sample=const.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 SFZ");
    let without = scratch.join("without-file.sfz");
    std::fs::write(
        &without,
        "<region> sample=missing.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 SFZ");

    let mut dispatcher = dispatcher_with_project();

    // ---- ① 装载：读到了 ⇒ 如实报 ----
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": with_file.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    assert_eq!(data["sampleFilesRead"], true, "{data}");
    assert_eq!(data["sampleFilesReadCount"], 1);
    assert_eq!(data["sampleFilesFailedCount"], 0);
    let source = data["sampleSource"].as_str().expect("sampleSource");
    assert!(source.starts_with("file("), "来源必须点名文件: {source}");
    assert!(
        source.contains("frames=48000 rate=48000"),
        "来源必须带帧数与采样率: {source}"
    );
    assert!(
        data["regionDetails"][0]["sampleFileState"]
            .as_str()
            .expect("state")
            .starts_with("file("),
        "逐区域也要如实: {data}"
    );

    // ---- ② 装载：指向不存在的文件 ⇒ 装载仍成功 + 如实回退 ----
    let missing = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": without.display().to_string() }),
    );
    assert_eq!(missing["status"], "success", "{missing}");
    assert_eq!(missing["data"]["sampleFilesRead"], false);
    assert!(
        missing["data"]["sampleSource"]
            .as_str()
            .expect("source")
            .starts_with("in-memory-sine"),
        "{missing}"
    );
    let state = missing["data"]["regionDetails"][0]["sampleFileState"]
        .as_str()
        .expect("state");
    assert!(state.starts_with("unreadable("), "{state}");
    assert!(
        state.contains("missing.wav"),
        "失败原因必须点名**解析后**的路径: {state}"
    );

    // ---- ③ 用文件样本渲染 ----
    let file_target = scratch.join("with-file.wav");
    let rendered_file = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": 1,
            "frames": 24_000,
            "path": file_target.display().to_string(),
        }),
    );
    assert_eq!(rendered_file["status"], "success", "{rendered_file}");
    let fd = &rendered_file["data"];
    assert_eq!(fd["sampleFilesRead"], true, "{fd}");
    assert!(
        fd["notesUsingFileSample"].as_u64().unwrap_or(0) > 0,
        "必须真的有音符用了文件样本: {fd}"
    );
    assert_eq!(fd["notesUsingSineFallback"], 0, "{fd}");

    // ---- ④ 回退正弦渲染（同一 SFZ 文本、同一音符、只差文件在不在）----
    let sine_target = scratch.join("with-sine.wav");
    let rendered_sine = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": 2,
            "frames": 24_000,
            "path": sine_target.display().to_string(),
        }),
    );
    assert_eq!(rendered_sine["status"], "success", "{rendered_sine}");
    let sd = &rendered_sine["data"];
    assert_eq!(sd["sampleFilesRead"], false, "{sd}");
    assert!(
        sd["notesUsingSineFallback"].as_u64().unwrap_or(0) > 0,
        "回退必须被计数: {sd}"
    );

    // ---- ⑤ 两侧都非静音 + 可区分（逐样本最大绝对差 ≠ 0）----
    let file_bytes = std::fs::read(&file_target).expect("读回文件侧");
    let sine_bytes = std::fs::read(&sine_target).expect("读回正弦侧");
    let (file_channels, file_rate, file_samples) = wav_read(&file_bytes);
    let (sine_channels, sine_rate, sine_samples) = wav_read(&sine_bytes);
    assert_eq!((file_channels, file_rate), (2, 48_000));
    assert_eq!((sine_channels, sine_rate), (2, 48_000));
    assert_eq!(file_samples.len(), sine_samples.len(), "同一请求 ⇒ 同长度");
    let peak = |samples: &[i32]| samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    let file_peak = peak(&file_samples);
    let sine_peak = peak(&sine_samples);
    let max_abs_diff = file_samples
        .iter()
        .zip(&sine_samples)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    println!(
        "[sfz-file] 文件侧 peak={file_peak} / 正弦侧 peak={sine_peak} / 最大绝对差={max_abs_diff}"
    );
    assert!(file_peak > 1000, "文件侧不许是静音（peak={file_peak}）");
    assert!(sine_peak > 1000, "正弦侧不许是静音（peak={sine_peak}）");
    assert!(
        max_abs_diff > 0,
        "两种样本来源必须产出**不同**的字节（最大绝对差 = {max_abs_diff}）"
    );

    // ---- ⑥ 输出**就是文件里的常量 0.25**：峰值平台极差 = 0，且振幅与 0.25 相符 ----
    //
    // 为什么不用"整段极差 = 0"：夹具工程里可能有不止一个命中音高的音符，而本渲染器
    // 对**每个音符末尾**做 64 帧线性淡出 ⇒ 任何"整段"极差都会含包络斜坡（那是包络，
    // 不是样本内容）。`app` 侧的 `SfzTrackSource` 没有这个淡出，所以它的读数是
    // `0.000000`；这里改成在**峰值平台**（|x| 等于峰值 ±1 的那些下标）上取极差 ——
    // 平台上没有斜坡，常量样本在平台上的极差必然是 0。
    let plateau: Vec<usize> = file_samples
        .iter()
        .enumerate()
        .filter(|(_, value)| value.unsigned_abs().abs_diff(file_peak) <= 1)
        .map(|(index, _)| index)
        .collect();
    assert!(
        plateau.len() > 1_000,
        "峰值平台太小（{}）⇒ 判据会退化成恒真",
        plateau.len()
    );
    let range_over = |samples: &[i32], indices: &[usize]| {
        let mut min = i32::MAX;
        let mut max = i32::MIN;
        for index in indices {
            let value = samples[*index];
            min = min.min(value);
            max = max.max(value);
        }
        (max - min, min, max)
    };
    let (file_range, file_min, file_max) = range_over(&file_samples, &plateau);
    let (sine_range, sine_min, sine_max) = range_over(&sine_samples, &plateau);
    println!(
        "[sfz-file] 峰值平台极差（{} 个样本，文件侧 peak={file_peak} / 正弦侧 peak={sine_peak}）: \
         文件侧 {file_range} (min={file_min} max={file_max}) / 正弦侧 {sine_range} (min={sine_min} max={sine_max})",
        plateau.len()
    );
    assert!(
        file_range <= 2,
        "常量样本在峰值平台上的极差必须 ≈ 0（实得 {file_range}）⇒ 输出就是文件里的那个常量"
    );
    assert!(
        sine_range > 100_000,
        "同一批下标在内存正弦侧必须明显不同（实得 {sine_range}）"
    );
    // ⭐ 定量的关系：文件样本是 0.25、内存正弦是 0.5 ⇒ 峰值之比 ≈ 2。
    let ratio = f64::from(sine_peak) / f64::from(file_peak);
    println!("[sfz-file] 峰值比（正弦/文件）= {ratio:.4}（期望 2.0 = 0.5 / 0.25）");
    assert!(
        (ratio - 2.0).abs() < 0.1,
        "峰值比必须落在 2.0 附近（0.5/0.25），实得 {ratio}"
    );
}

#[test]
fn the_load_response_reports_the_file_it_really_read() {
    let scratch = Scratch::new("dir");
    // ⚠ 子目录：SFZ 在 `sub/` 里，样本与它**同目录**；进程 cwd 不是那里 ⇒
    // "相对 SFZ 文件所在目录解析"这条被真的测到。
    let sub = scratch.join("sub");
    std::fs::create_dir_all(&sub).expect("建子目录");
    write_constant_wav(&sub.join("tone.wav"), 0.5, 4_800, 44_100);
    let sfz = sub.join("in-sub.sfz");
    std::fs::write(&sfz, "<region> sample=tone.wav key=60 pitch_keycenter=60\n").expect("写 SFZ");

    let mut dispatcher = dispatcher_with_project();
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    assert_eq!(data["sampleFilesRead"], true, "{data}");
    let source = data["sampleSource"].as_str().expect("source");
    println!("[sfz-file] 子目录相对路径来源: {source}");
    assert!(
        source.contains("tone.wav")
            && source.contains("frames=4800")
            && source.contains("rate=44100"),
        "相对 SFZ 目录解析后的路径/帧数/采样率必须如实: {source}"
    );
    assert_eq!(data["sampleFramesLoaded"], 4_800, "累计解码帧数必须如实");
}

#[test]
fn an_oversized_sample_file_is_refused_loudly_and_does_not_break_the_load() {
    let scratch = Scratch::new("oversize");
    // **稀疏**文件：`set_len` 不写数据（APFS 支持），因此造一个 > 64 MiB 的"文件"是廉价的。
    let big = scratch.join("big.wav");
    let file = std::fs::File::create(&big).expect("建大文件");
    file.set_len(64 * 1024 * 1024 + 1).expect("置长度");
    drop(file);

    let sfz = scratch.join("big.sfz");
    std::fs::write(&sfz, "<region> sample=big.wav key=60 pitch_keycenter=60\n").expect("写 SFZ");

    let mut dispatcher = dispatcher_with_project();
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(
        loaded["status"], "success",
        "上界命中**不是**装载失败（如实回退）: {loaded}"
    );
    let data = &loaded["data"];
    assert_eq!(data["sampleFilesRead"], false, "{data}");
    let state = data["regionDetails"][0]["sampleFileState"]
        .as_str()
        .expect("state");
    println!("[sfz-file] 超限样本的如实回退: {state}");
    assert!(state.starts_with("unreadable("), "{state}");
    assert!(
        state.contains("超过单文件上限"),
        "原因必须说清是**上界**（⛔ 不是静默截断）: {state}"
    );
    assert_eq!(data["sampleFramesLoaded"], 0, "一个字节都不许读进来");
}
