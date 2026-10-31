//! ⭐ 任务单 `#218`：MCP 的两个 SFZ 工具认 `sample=*silence`（内建静音伪样本）。
//!
//! ## 量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 来源 | 单位 |
//! | :--- | :--- | :--- |
//! | `silent` | `data.silentRegionCount`（`yeban_load_sfz`） | **引擎摊平口径**的静音区域数（缺样本 ＋ 内建静音；报告缺口 `G4`） |
//! | `builtinSilent` | `data.builtinSilentRegionCount`（`yeban_load_sfz`） | 其中 `sample=*silence` 的子集 |
//! | `notesBuiltinSilence` | `data.notesUsingBuiltinSilence`（`yeban_render_sfz_wav`） | 落在内建静音区域上的**音符数** |
//! | `samples` | `data.sampleFilesReadCount` | 真的解出 PCM 的样本**份数** |
//! | `missing` / `unreadable` | `data.sampleFailures` / `data.sampleFilesFailedCount` | 读盘失败**条数**（⛔ 不含 `*silence`） |
//! | `energy` / `peak` | 回读导出 WAV 的 i24 样本 | `Σx²` / `max|x|`（`≤ 1 LSB` ＝ 抖动底，不是信号） |
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证用户机器上的真库（`~/Library/.../Samples`）——
//! 夹具全部由本测试自己写在 `std::env::temp_dir()` 下（`Drop` 删除）。
//!
//! ## 判据有牙（反向对照）
//!
//! 同一个夹具里有**一条真样本区域**（`sample=tone.wav`，常量 `0.25` 的 24-bit WAV）：
//! 它必须真的被读、被计数、渲染出声（`peak > 0`）。⇒ 静音侧的 `0` ⛔ 不是
//! "整条夹具都不匹配" 的假象；而 `note: 1`（夹具工程里**没有**这个音高）另给一条
//! "排程数 0"的负对照，用来区分"内建静音"与"没有音符"。
//!
//! ⚠ 第 323 条口径：能量判据**只**用在"⭐ 静音"这一条（目标值就是 0，且配正对照）。
//! "有没有解码 / 有没有读盘"一律不用能量判，用 `sampleFailures` / `sampleFilesRead` /
//! `notesUsingBuiltinSilence` 三个计数。
//!
//! ## 装配
//!
//! 判据走**生产形状**的 `Dispatcher`（`RunMode::Production` + `Channel::Http` 的
//! `tools/call`）—— 与 `yeban-mcp --stdio` 和 app 的内嵌 `session_dispatcher`
//! 是**同一个** `Dispatcher` / `Domain` / `domain::sfz`（第 321 条：那两条装配的差别只在
//! `read_only` 与 `set_now_ms`，都不参与本工具的装载/渲染）。

use std::path::{Path, PathBuf};

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{ClipContent, EntityId, YebanProjectV1};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-sfzsilence-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn write_sfz(&self, name: &str, body: &str) -> PathBuf {
        let path = self.join(name);
        std::fs::write(&path, body).expect("写 SFZ 夹具");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// **手写**一份常量值的 24-bit 单声道 WAV（⛔ 不用被测写入器自证）。
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

/// 判据夹具的工程：`filled_project()`，但把全部音符的概率触发清掉。
///
/// ⚠ 为什么必须清：`filled_project()` 的第 4 个音符（`pitch=72`）带
/// `probability=Some(0.75)` ⇒ 在固定 `rng_seed` 下**恰好不触发**（`notesSilenced == 1`），
/// 拿它当"真样本正对照"会让判据红在"没有音符被排程"，而不是红在被测行为。
/// 清掉概率后，`pitch=72` 必然触发；`pitch=64` 的 `ratchet=2` 仍会展开成 2 个脉冲。
fn project_without_note_probability() -> YebanProjectV1 {
    let mut project = filled_project();
    for clip in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut clip.content {
            for note in notes.values_mut() {
                note.probability = None;
            }
        }
    }
    project
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-sfzsilence-{}.yeban", std::process::id())),
            project_without_note_probability(),
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

fn peak(samples: &[i32]) -> i32 {
    samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0) as i32
}

fn energy(samples: &[i32]) -> i128 {
    samples
        .iter()
        .map(|sample| {
            let value = i128::from(*sample);
            value * value
        })
        .sum()
}

/// 24-bit 的 1 LSB（解码腿按 `Int24 / 8_388_607` 归一 ⇒ 抖动底是 1 个 i24 单位）。
const ONE_LSB_I24: i32 = 1;

/// 夹具 SFZ 文本 —— 与 app 侧 `sfz_silence_pseudo_sample.rs` 的 `mixed.sfz` **同一形状**
/// （`key=60 sample=*silence` ＋ `key=72 sample=tone.wav`）。
const MIXED_SFZ: &str = "<region> key=60 sample=*silence\n<region> key=72 sample=tone.wav\n";

// ---------------------------------------------------------------------------
// 判据 ①：装载 —— `*silence` 进 `silent`、⛔ 不进 `missing`/`unreadable`
// ---------------------------------------------------------------------------

#[test]
fn load_sfz_counts_the_builtin_silence_and_never_reports_it_as_a_missing_file() {
    let scratch = Scratch::new("load");
    write_constant_wav(&scratch.join("tone.wav"), 0.25, 480, 48_000);
    let mixed = scratch.write_sfz("mixed.sfz", MIXED_SFZ);
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": mixed.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    let failures = data["sampleFailures"].as_array().expect("sampleFailures");
    println!(
        "[mcp/silence/load] regions={} silent={} samples={} missing={:?} \
         failed_count={} read_count={} sampleSource={:?} region0_state={:?} region1_state={:?}",
        data["regions"],
        data["silentRegionCount"],
        data["sampleFilesReadCount"],
        failures,
        data["sampleFilesFailedCount"],
        data["sampleFilesReadCount"],
        data["sampleSource"],
        data["regionDetails"][0]["sampleFileState"],
        data["regionDetails"][1]["sampleFileState"],
    );

    assert_eq!(data["regions"], 2, "区域数: {data}");
    assert_eq!(
        data["silentRegionCount"], 1,
        "`*silence` 必须计入 silent（摊平口径：本夹具另一条读了文件 ⇒ 只此 1 条）: {data}"
    );
    assert_eq!(
        data["builtinSilentRegionCount"], 1,
        "`*silence` 是内建静音子集: {data}"
    );
    assert!(
        failures.is_empty(),
        "`*silence` ⛔ 不是缺文件; 实得 {failures:?}"
    );
    assert_eq!(
        data["sampleFilesFailedCount"], 0,
        "`*silence` ⛔ 不是 unreadable: {data}"
    );
    assert_eq!(
        data["sampleFilesReadCount"], 1,
        "只有 tone.wav 这一份真样本: {data}"
    );
    assert_eq!(data["sampleFilesRead"], true);
    assert!(
        data["sampleSources"][0]
            .as_str()
            .expect("sampleSources[0]")
            .contains("tone.wav"),
        "读到的必须是 tone.wav: {data}"
    );
    // 逐区域：第 0 条是内建静音（⛔ 不是 `unreadable(...)`），第 1 条是真文件。
    assert_eq!(data["regionDetails"][0]["sample"], "*silence");
    assert_eq!(data["regionDetails"][0]["builtinSilence"], true);
    let state0 = data["regionDetails"][0]["sampleFileState"]
        .as_str()
        .expect("region0 state");
    assert_eq!(
        state0, "builtin-silence(*silence)",
        "内建静音的区域状态逐字（⛔ 不是 unreadable）: {data}"
    );
    assert_eq!(data["regionDetails"][1]["builtinSilence"], false);
    let state1 = data["regionDetails"][1]["sampleFileState"]
        .as_str()
        .expect("region1 state");
    assert!(
        state1.starts_with("file(") && state1.contains("tone.wav"),
        "真样本区域必须读到文件: {state1}"
    );

    // 只含 `*silence` 的文件：来源文案必须逐字是内建静音（⛔ 不回退内存正弦）。
    let only = scratch.write_sfz("only-silence.sfz", "<region> key=60 sample=*silence\n");
    let only_loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": only.display().to_string() }),
    );
    assert_eq!(only_loaded["status"], "success", "{only_loaded}");
    let only_data = &only_loaded["data"];
    println!(
        "[mcp/silence/load-only] regions={} silent={} missing={:?} sampleSource={:?}",
        only_data["regions"],
        only_data["silentRegionCount"],
        only_data["sampleFailures"],
        only_data["sampleSource"],
    );
    assert_eq!(only_data["regions"], 1);
    assert_eq!(only_data["silentRegionCount"], 1);
    assert_eq!(only_data["builtinSilentRegionCount"], 1);
    assert!(
        only_data["sampleFailures"]
            .as_array()
            .expect("sampleFailures")
            .is_empty(),
        "只有 `*silence` 时也必须零失败: {only_data}"
    );
    assert_eq!(
        only_data["sampleSource"], "builtin-silence(*silence)",
        "来源必须是内建静音（⛔ 不是 in-memory-sine）: {only_data}"
    );

    // ⭐ `default_path` 前缀下的判据：#217 的裁决是判 `sample=` **原值**。
    // `<control> default_path=samples/` ＋ `sample=*silence` 仍是内建静音（⛔ 不读
    // `samples/*silence`）；`regionDetails[].sample` 也必须是**原值**（`#218` 顺手把
    // 摘要从 `sample_path()` 改回 `Region::sample` —— 字段契约本来就写着"原样登记"）。
    let prefixed = scratch.write_sfz(
        "prefixed.sfz",
        "<control> default_path=samples/\n<region> key=60 sample=*silence\n",
    );
    let prefixed_loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": prefixed.display().to_string() }),
    );
    assert_eq!(prefixed_loaded["status"], "success", "{prefixed_loaded}");
    let pd = &prefixed_loaded["data"];
    println!(
        "[mcp/silence/load-prefixed] regions={} silent={} missing={:?} sample={:?} state={:?}",
        pd["regions"],
        pd["silentRegionCount"],
        pd["sampleFailures"],
        pd["regionDetails"][0]["sample"],
        pd["regionDetails"][0]["sampleFileState"],
    );
    assert_eq!(pd["regions"], 1);
    assert_eq!(pd["silentRegionCount"], 1);
    assert_eq!(pd["builtinSilentRegionCount"], 1);
    assert!(
        pd["sampleFailures"]
            .as_array()
            .expect("sampleFailures")
            .is_empty(),
        "`default_path` 下的 `*silence` 也不得被当成缺文件: {pd}"
    );
    assert_eq!(
        pd["regionDetails"][0]["sample"], "*silence",
        "`sample=` 字段必须是原值（⛔ 不是拼过 default_path 的 samples/*silence）: {pd}"
    );
}

/// 判据 ①b（报告缺口 `G4`）：**读不到的样本也计入 `silentRegionCount`** ——
/// 与 `yeban_render_master` 的 `sampler.silentRegionCount` 同口径；且回退宣称带适用范围。
///
/// 修前：装载回执只数内建静音 ⇒ 同一份"全部样本读不到"的乐器被报成 0，
/// 而绑轨渲染报 `silent=360` 且音频是数字静音 —— 三处表述互相矛盾
/// （Zimmer 2026-10-31 报告 §5.3）。本判据在**装载侧**钉住摊平口径与回退边界；
/// 与母带回执的逐值对账在 `render_master.rs` 的同名 `g4` 判据里。
#[test]
fn unreadable_samples_count_toward_silent_region_count_and_the_fallback_says_where_it_applies() {
    let scratch = Scratch::new("g4-silent");
    let dead = scratch.write_sfz(
        "dead.sfz",
        "<region> key=60 sample=gone-a.wav\n<region> key=61 sample=gone-b.wav\n",
    );
    let mut dispatcher = dispatcher_with_project();
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": dead.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    println!(
        "[mcp/silence/g4] regions={} silent={} builtin={} failures={:?} sampleSource={:?}",
        data["regions"],
        data["silentRegionCount"],
        data["builtinSilentRegionCount"],
        data["sampleFailures"],
        data["sampleSource"],
    );
    assert_eq!(
        data["silentRegionCount"], 2,
        "两条区域都读不到 ⇒ 摊平口径都是静音（⛔ 修前的 0）: {data}"
    );
    assert_eq!(
        data["builtinSilentRegionCount"], 0,
        "没有 `*silence` ⇒ 内建静音子集为 0: {data}"
    );
    assert_eq!(
        data["sampleFailures"]
            .as_array()
            .expect("sampleFailures")
            .len(),
        2,
        "两条失败必须逐条登记: {data}"
    );
    // ⭐ 回退宣称必须带适用范围：正弦只属于 `yeban_render_sfz_wav` 试听路径。
    let source = data["sampleSource"].as_str().expect("sampleSource");
    assert!(
        source.starts_with("in-memory-sine"),
        "⛔ 不许改掉前缀（既有判据按它断言）: {source}"
    );
    assert!(
        source.contains("采样器轨不回退"),
        "回退文案必须点名适用边界（否则会被读成母带也有声）: {source}"
    );
}

// ---------------------------------------------------------------------------
// 判据 ②③：导出 —— 内建静音真的静音；同一夹具有正对照
// ---------------------------------------------------------------------------

#[test]
fn render_sfz_wav_is_really_silent_on_the_builtin_silence_and_audible_on_the_real_sample() {
    let scratch = Scratch::new("render");
    write_constant_wav(&scratch.join("tone.wav"), 0.25, 480, 48_000);
    let mixed = scratch.write_sfz("mixed.sfz", MIXED_SFZ);
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": mixed.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let instrument_id = loaded["data"]["instrumentId"]
        .as_u64()
        .expect("instrumentId");

    // ---- ① `note: 60` ⇒ 只落在内建静音区域 ----
    let silence_wav = scratch.join("silence-only.wav");
    let silent = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": silence_wav.display().to_string(),
            "note": 60,
        }),
    );
    assert_eq!(silent["status"], "success", "{silent}");
    let sd = &silent["data"];
    assert_eq!(sd["notesScheduled"], 1, "工程里恰好一个 60: {sd}");
    assert_eq!(
        sd["notesUsingBuiltinSilence"], 1,
        "落在内建静音区域上的音符数: {sd}"
    );
    assert_eq!(sd["notesUsingSineFallback"], 0, "⛔ 不许回退内存正弦: {sd}");
    assert_eq!(sd["notesUsingFileSample"], 0, "⛔ 没有文件样本可播: {sd}");
    let failures = sd["sampleFailures"].as_array().expect("sampleFailures");
    assert!(
        failures.is_empty(),
        "⛔ 不许把 `*silence` 记成失败: {failures:?}"
    );
    assert_eq!(sd["sampleSource"], "builtin-silence(*silence)", "{sd}");

    let bytes = std::fs::read(&silence_wav).expect("读回静音 WAV");
    let (channels, rate, samples) = wav_read(&bytes);
    println!(
        "[mcp/silence/render/silence] channels={channels} rate={rate} samples={} peak={} energy={}",
        samples.len(),
        peak(&samples),
        energy(&samples)
    );
    assert_eq!((channels, rate), (2, 48_000));
    assert_eq!(samples.len(), 4_800 * 2, "帧数 × 声道数");
    assert!(
        peak(&samples) <= ONE_LSB_I24,
        "内建静音只应有抖动底 (≤ 1 LSB), 实得 {}",
        peak(&samples)
    );
    // 抖动底的能量上界：每样本 ≤ 1 LSB²。
    assert!(
        energy(&samples)
            <= i128::from(ONE_LSB_I24) * i128::from(ONE_LSB_I24) * samples.len() as i128,
        "内建静音的能量必须 ≤ 抖动底, 实得 {}",
        energy(&samples)
    );

    // ---- ② 正对照：`note: 72` ⇒ 真样本区域必须出声 ----
    let tone_wav = scratch.join("tone-only.wav");
    let tone = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            // 128 BPM / 960 PPQ：`pitch=72` 的起点是 tick 2880 = 67,500 帧 @48 kHz
            // ⇒ 输出窗口必须覆盖它（4,800 帧只到 0.1 秒，正对照会假静音）。
            "frames": 96_000,
            "path": tone_wav.display().to_string(),
            "note": 72,
        }),
    );
    assert_eq!(tone["status"], "success", "{tone}");
    let td = &tone["data"];
    assert_eq!(td["notesUsingBuiltinSilence"], 0, "{td}");
    assert_eq!(td["notesUsingFileSample"], 1, "真样本必须被用上: {td}");
    assert_eq!(td["notesUsingSineFallback"], 0, "{td}");
    let tone_bytes = std::fs::read(&tone_wav).expect("读回正对照 WAV");
    let (_, _, tone_samples) = wav_read(&tone_bytes);
    println!(
        "[mcp/silence/render/tone] samples={} peak={} energy={}",
        tone_samples.len(),
        peak(&tone_samples),
        energy(&tone_samples)
    );
    assert!(
        peak(&tone_samples) > 1_000,
        "正对照：真样本区域必须出声（峰值实得 {}）",
        peak(&tone_samples)
    );
    assert!(energy(&tone_samples) > 0);

    // ---- ③ 负对照：`note: 1` ⇒ 夹具工程里没有这个音高（排程 0，⛔ 不是静音区域）----
    let unmatched_wav = scratch.join("unmatched.wav");
    let unmatched = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 1_200,
            "path": unmatched_wav.display().to_string(),
            "note": 1,
        }),
    );
    assert_eq!(unmatched["status"], "success", "{unmatched}");
    assert_eq!(unmatched["data"]["notesScheduled"], 0, "{unmatched}");
    assert_eq!(unmatched["data"]["notesUsingBuiltinSilence"], 0);
    let unmatched_bytes = std::fs::read(&unmatched_wav).expect("读回负对照 WAV");
    let (_, _, unmatched_samples) = wav_read(&unmatched_bytes);
    assert!(
        peak(&unmatched_samples) <= ONE_LSB_I24,
        "没有音符命中 ⇒ 只有抖动底, 实得 {}",
        peak(&unmatched_samples)
    );

    // ---- ④ 整工程一次：静音区域与真样本区域**同时**在排程里，读数分别计数 ----
    let mixed_wav = scratch.join("mixed.wav");
    let both = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            // 整工程：最后一个音符（72）在 67,500 帧 ⇒ 窗口要够长。
            "frames": 96_000,
            "path": mixed_wav.display().to_string(),
        }),
    );
    assert_eq!(both["status"], "success", "{both}");
    let bd = &both["data"];
    println!(
        "[mcp/silence/render/mixed] scheduled={} unmatched={} builtin_silence={} file={} sine={} \
         failures={:?} sampleSource={:?}",
        bd["notesScheduled"],
        bd["notesUnmatched"],
        bd["notesUsingBuiltinSilence"],
        bd["notesUsingFileSample"],
        bd["notesUsingSineFallback"],
        bd["sampleFailures"],
        bd["sampleSource"],
    );
    assert_eq!(
        bd["notesScheduled"], 5,
        "工程里的 4 个音符（64 带 ratchet=2）: {bd}"
    );
    assert_eq!(bd["notesUsingBuiltinSilence"], 1, "60 落在内建静音上: {bd}");
    assert_eq!(bd["notesUsingFileSample"], 1, "72 用真样本: {bd}");
    assert_eq!(
        bd["notesUnmatched"], 3,
        "64 的两个脉冲 ＋ 67 都没有区域: {bd}"
    );
    assert_eq!(bd["notesSilenced"], 0, "概率已清 ⇒ 没有概率静音: {bd}");
    assert!(
        bd["sampleFailures"]
            .as_array()
            .expect("sampleFailures")
            .is_empty(),
        "`*silence` ⛔ 不进失败表: {bd}"
    );
    let mixed_bytes = std::fs::read(&mixed_wav).expect("读回整工程 WAV");
    let (_, _, mixed_samples) = wav_read(&mixed_bytes);
    println!(
        "[mcp/silence/render/mixed-wav] samples={} peak={} energy={}",
        mixed_samples.len(),
        peak(&mixed_samples),
        energy(&mixed_samples)
    );
    assert!(
        peak(&mixed_samples) > 1_000,
        "整工程导出必须出声（正对照仍在）: {}",
        peak(&mixed_samples)
    );
}
