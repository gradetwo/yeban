//! ⭐ 空排程诊断（补丁 `1dd0e332f`）：`yeban_render_sfz_wav` 在窗口内零音符时，
//! 回执必须带 `notesEmpty{reason, 计数, hint}`，且正常路径一个键都不加。
//!
//! ## 量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 来源 | 单位 |
//! | :--- | :--- | :--- |
//! | `reason` | `data.notesEmpty.reason` | 五值之一（见下表） |
//! | `notesInProject` / `notesScheduled` / `notesInWindow` | `data.notesEmpty.*` | **个**音符 |
//! | `silencedByProbability` | `data.notesEmpty.silencedByProbability` | **个**音符 |
//! | `hint` | `data.notesEmpty.hint` | **非空**文案，必须点名"已排程的 MIDI 音符" |
//! | `peak` | `data.peak` | 恒 `0.0`（空排程 ⇒ 全静音） |
//! | 导出 WAV | 回读落盘文件 | 峰值 `≤ 1 LSB`（抖动底，不是信号） |
//!
//! ## 五值判定（与 `domain/mod.rs` 里 `plan_render_sfz_wav` 同一条分支顺序）
//!
//! | `reason` | 条件 |
//! | :--- | :--- |
//! | `projectHasNoMidiNotes` | 工程里一个 MIDI 音符都没有 |
//! | `allNotesSilencedByProbability` | 有音符，但全被概率门静音 |
//! | `pitchNotPresentInProject` | 有音符，但 `note` 过滤器筛掉全部 |
//! | `noSchedulableNotes` | 有音符、无过滤、无静音，但排程仍为空（本文件用"全部片段静音"构造） |
//! | `allScheduledNotesOutsideWindow` | 音符都排在输出窗口之后 |
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证"正常路径逐字节不变"（那是补丁 `1dd0e332f` 的自述，
//! 逐字节比对属于另一条判据）；⛔ 不证用户机器上的真库 —— 夹具全部由本测试
//! 自己写在 `std::env::temp_dir()` 下（`Drop` 删除）。
//!
//! ## 编号说明
//!
//! 补丁 `1dd0e332f` 自述"新增判据 S119"，但全树核查证实：没有任何测试提到
//! `notesEmpty`，且 `S119` 这个编号**已被占用**（`yeban-engine` 的
//! `sfz_instrument.rs` 里 S119 ＝ 旁路 CC 播种）。为避免重号，本文件不用
//! S 编号，用描述性判据名，对应补丁写在文件头这一处。
//!
//! ## 装配
//!
//! 判据走**生产形状**的 `Dispatcher`（`RunMode::Production` + `Channel::Http` 的
//! `tools/call`），与 `sfz_silence_sample.rs` 同一套夹具与调用口径。

use std::path::{Path, PathBuf};

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::{filled_project, project_with_notes};
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
            "yeban-mcp-sfzempty-{}-{tag}-{}",
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
/// `probability=Some(0.75)` ⇒ 在固定 `rng_seed` 下**恰好不触发**，拿它当
/// "真样本正对照"会让判据红在"没有音符被排程"，而不是红在被测行为。
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

/// 全部音符都被概率门静音：`probability=Some(0.0)` 在 `MidiNote::triggers`
/// 里是**恒 false**（与种子无关），不是"恰好不触发"的运气。
fn project_all_silenced_by_probability() -> YebanProjectV1 {
    let mut project = filled_project();
    for clip in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut clip.content {
            for note in notes.values_mut() {
                note.probability = Some(0.0);
            }
        }
    }
    project
}

/// 全部片段静音：`collect_notes` 会跳过 `muted` 的片段，但
/// `count_project_midi_notes` **不看** `muted` —— 于是排程为空而工程里
/// 仍有音符，正好落进 `noSchedulableNotes` 这一支。
fn project_all_placements_muted() -> YebanProjectV1 {
    let mut project = project_without_note_probability();
    for track in project.tracks.values_mut() {
        for placement in track.clips.values_mut() {
            placement.muted = true;
        }
    }
    project
}

/// 全部音符都排在输出窗口之后：把片段起点推到 tick 7680（@48 kHz、128 BPM、
/// 960 PPQ ⇒ 180,000 帧），`frames` 只给 4,800 帧时一个音符都落不进窗口。
/// 这正是产品注释里的第二种成因（"工程第一句在 32 s，而 `frames` 只给了 2 s"）。
fn project_notes_delayed() -> YebanProjectV1 {
    let mut project = project_without_note_probability();
    for track in project.tracks.values_mut() {
        for placement in track.clips.values_mut() {
            placement.start_tick = 7680;
        }
    }
    project
}

fn dispatcher_with_project(project: YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-sfzempty-{}.yeban", std::process::id())),
            project,
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

/// 24-bit 的 1 LSB（解码腿按 `Int24 / 8_388_607` 归一 ⇒ 抖动底是 1 个 i24 单位）。
const ONE_LSB_I24: i32 = 1;

/// 夹具 SFZ 文本 —— 与 `sfz_silence_sample.rs` 的 `mixed.sfz` **同一形状**
/// （`key=60 sample=*silence` ＋ `key=72 sample=tone.wav`）。
const MIXED_SFZ: &str = "<region> key=60 sample=*silence\n<region> key=72 sample=tone.wav\n";

/// 装载夹具 SFZ，返回 `instrumentId`。
fn load_mixed_sfz(dispatcher: &mut Dispatcher, scratch: &Scratch) -> u64 {
    write_constant_wav(&scratch.join("tone.wav"), 0.25, 480, 48_000);
    let mixed = scratch.write_sfz("mixed.sfz", MIXED_SFZ);
    let loaded = call_tool(
        dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": mixed.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    loaded["data"]["instrumentId"]
        .as_u64()
        .expect("instrumentId")
}

/// 渲染一次，返回回执 `data` 与落盘 WAV 路径。
fn render(
    dispatcher: &mut Dispatcher,
    instrument_id: u64,
    frames: u64,
    path: &Path,
    note: Option<u8>,
) -> Value {
    let mut args = serde_json::json!({
        "instrumentId": instrument_id,
        "frames": frames,
        "path": path.display().to_string(),
    });
    if let Some(pitch) = note {
        args["note"] = serde_json::json!(pitch);
    }
    let receipt = call_tool(dispatcher, "yeban_render_sfz_wav", args);
    assert_eq!(receipt["status"], "success", "{receipt}");
    receipt["data"].clone()
}

/// `notesEmpty` 的结构断言：reason、计数、hint 一处都不少。
/// `note_filter`：`None` ＝ 未传 `note`（回执里是 `null`），`Some(p)` ＝ 传了该音高。
/// `notes_scheduled`：⚠ 五值里只有 `allScheduledNotesOutsideWindow` 不是 0 ——
/// 音符确实被收集排程了（本文件该例为 5），只是全落在输出窗口之后；
/// 其余四值都源于"一个音符都没收上来"，排程恒 0。
fn assert_notes_empty(
    data: &Value,
    reason: &str,
    notes_in_project: u64,
    notes_scheduled: u64,
    silenced_by_probability: u64,
    note_filter: Option<u8>,
    frames: u64,
) {
    let empty = data.get("notesEmpty").expect("空排程必须带 notesEmpty");
    assert_eq!(empty["reason"], reason, "reason 必须是指定的那一值: {data}");
    assert_eq!(
        empty["notesInProject"], notes_in_project,
        "工程里的原始 MIDI 音符数: {data}"
    );
    assert_eq!(
        empty["notesScheduled"], notes_scheduled,
        "排程数（窗口外一例除外，其余恒 0）: {data}"
    );
    assert_eq!(empty["notesInWindow"], 0, "空排程 ⇒ 窗口内 0: {data}");
    assert_eq!(
        empty["frames"], frames,
        "回执的帧数必须是请求的帧数: {data}"
    );
    match note_filter {
        Some(pitch) => assert_eq!(empty["noteFilter"], pitch, "过滤器音高必须原样回报: {data}"),
        None => assert!(
            empty["noteFilter"].is_null(),
            "没传 `note` ⇒ noteFilter 必须是 null: {data}"
        ),
    }
    assert_eq!(
        empty["silencedByProbability"], silenced_by_probability,
        "被概率门静音的音符数: {data}"
    );
    let hint = empty["hint"].as_str().expect("hint 必须是字符串");
    assert!(!hint.is_empty(), "hint 不得为空: {data}");
    assert!(
        hint.contains("已排程的 MIDI 音符"),
        "hint 必须点名诊断口径（`note` 是过滤器不是试听器）: {hint}"
    );
    assert_eq!(
        data["notesScheduled"], notes_scheduled,
        "顶层排程数与 notesEmpty 里同一值: {data}"
    );
    assert_eq!(
        data["peak"].as_f64(),
        Some(0.0),
        "空排程 ⇒ 输出全静音，峰值 0.0: {data}"
    );
}

/// 落盘 WAV 必须是合法的全静音：帧数 × 声道数都对，峰值只有抖动底。
fn assert_wav_is_silent(path: &Path, frames: u64) {
    let bytes = std::fs::read(path).expect("读回导出 WAV");
    let (channels, rate, samples) = wav_read(&bytes);
    assert_eq!((channels, rate), (2, 48_000), "恒 48 kHz 立体声");
    assert_eq!(
        samples.len(),
        (frames * 2) as usize,
        "帧数 × 声道数必须与请求一致"
    );
    assert!(
        peak(&samples) <= ONE_LSB_I24,
        "空排程只应有抖动底 (≤ 1 LSB), 实得 {}",
        peak(&samples)
    );
}

// ---------------------------------------------------------------------------
// 判据：五值各一例 ＋ 正常路径正对照
// ---------------------------------------------------------------------------

#[test]
fn empty_project_reports_project_has_no_midi_notes() {
    let scratch = Scratch::new("no-midi");
    let mut dispatcher = dispatcher_with_project(project_with_notes(0));
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("empty.wav");
    let data = render(&mut dispatcher, instrument_id, 4_800, &out, None);
    assert_notes_empty(&data, "projectHasNoMidiNotes", 0, 0, 0, None, 4_800);
    assert_wav_is_silent(&out, 4_800);
}

#[test]
fn all_notes_silenced_by_probability() {
    let scratch = Scratch::new("silenced");
    let mut dispatcher = dispatcher_with_project(project_all_silenced_by_probability());
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("silenced.wav");
    let data = render(&mut dispatcher, instrument_id, 96_000, &out, None);
    // 工程里 4 个原始音符全被概率门静音（`Some(0.0)` 恒不触发，与种子无关）。
    assert_notes_empty(
        &data,
        "allNotesSilencedByProbability",
        4,
        0,
        4,
        None,
        96_000,
    );
    assert_wav_is_silent(&out, 96_000);
}

#[test]
fn pitch_filter_with_no_match_reports_pitch_not_present_in_project() {
    let scratch = Scratch::new("pitch");
    let mut dispatcher = dispatcher_with_project(project_without_note_probability());
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("pitch.wav");
    // `note: 1` —— 夹具工程里没有这个音高（与 `sfz_silence_sample.rs` 的负对照同一构造）。
    let data = render(&mut dispatcher, instrument_id, 1_200, &out, Some(1));
    assert_notes_empty(&data, "pitchNotPresentInProject", 4, 0, 0, Some(1), 1_200);
    assert_wav_is_silent(&out, 1_200);
}

#[test]
fn muted_placements_report_no_schedulable_notes() {
    let scratch = Scratch::new("muted");
    let mut dispatcher = dispatcher_with_project(project_all_placements_muted());
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("muted.wav");
    let data = render(&mut dispatcher, instrument_id, 96_000, &out, None);
    assert_notes_empty(&data, "noSchedulableNotes", 4, 0, 0, None, 96_000);
    assert_wav_is_silent(&out, 96_000);
}

#[test]
fn notes_after_window_report_all_scheduled_notes_outside_window() {
    let scratch = Scratch::new("window");
    let mut dispatcher = dispatcher_with_project(project_notes_delayed());
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("window.wav");
    // 片段起点 tick 7680 ⇒ 首音符在 180,000 帧；窗口只给 4,800 帧。
    let data = render(&mut dispatcher, instrument_id, 4_800, &out, None);
    // 排程数是 5：音符确实被收集了，只是全落在窗口之后（`notesInWindow` 仍为 0）。
    assert_notes_empty(
        &data,
        "allScheduledNotesOutsideWindow",
        4,
        5,
        0,
        None,
        4_800,
    );
    assert_wav_is_silent(&out, 4_800);
}

#[test]
fn normal_path_has_no_notes_empty() {
    let scratch = Scratch::new("normal");
    let mut dispatcher = dispatcher_with_project(project_without_note_probability());
    let instrument_id = load_mixed_sfz(&mut dispatcher, &scratch);
    let out = scratch.join("normal.wav");
    let data = render(&mut dispatcher, instrument_id, 96_000, &out, None);
    assert!(
        data.get("notesEmpty").is_none(),
        "有音符的正常路径一个键都不许加: {data}"
    );
    // 正对照：`pitch=64` 带 ratchet=2 ⇒ 5 个排程音符，与 `sfz_silence_sample.rs` 的整工程一条同数。
    assert_eq!(data["notesScheduled"], 5, "{data}");
    let bytes = std::fs::read(&out).expect("读回正对照 WAV");
    let (_, _, samples) = wav_read(&bytes);
    assert_eq!(samples.len(), 96_000 * 2, "帧数 × 声道数");
    assert!(
        peak(&samples) > 1_000,
        "正对照必须真的出声（峰值实得 {}）",
        peak(&samples)
    );
}
