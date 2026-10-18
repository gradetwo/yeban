//! 判据（任务单 `#223`）：MCP 的采样解码**不再只认 PCM WAV**。
//!
//! ## 修的是什么（`#220`／`#221` 的实测事实）
//!
//! `crates/yeban-mcp/src/domain/sfz.rs` 的 `read_plain_wav_mono_f32` 只读 PCM WAV
//! ⇒ 真实库 `freepats-spanish-classical-guitar`（**FLAC**）的 48/48 区域全部失败
//! （逐字 `hound 失败: Ill-formed WAVE file: no RIFF tag found`）⇒ 渲染回退内存正弦。
//! 现在与 app 的 `sfz_load::read_sample_file` **同一条分派**：`.wav`/`.wave` 走既有
//! hound 腿，其余走 `yeban_decode::decode_mono_channel0`（symphonia ＋ 声道 0 折叠）。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `sampleFilesReadCount` | 本次装载真的读到的**不同样本文件** | 个 |
//! | `sampleFilesFailedCount` | 读失败的**不同样本文件** | 个 |
//! | `sampleFramesLoaded` | 累计解码帧数（所有文件之和） | 帧 |
//! | `sampleSource` | 如实来源串：`file(<path> frames=N rate=R)` 或回退原因 | 字符串 |
//! | `notesUsingFileSample` | 用文件样本发声的音符 | 个 |
//! | `notesUsingSineFallback` | 回退内存正弦的音符 | 个 |
//!
//! ## 夹具（⛔ 不把音频样本入库）
//!
//! FLAC／OGG 字节由 `yeban-decode` 的 `testfix` **用代码生成**：本 crate 的
//! `[dev-dependencies]` 以 `test-fixtures` feature 打开它。生成器本身的合法性由
//! `yeban-decode` 自己的判据钉住（`testfix` 模块文档：零第三方依赖，可独立编译）。
//! WAV 仍是**手写字节**（与 `sfz_sample_files.rs` 同一份常量样本口径）。

use std::path::{Path, PathBuf};

use serde_json::Value;
use yeban_decode::testfix::{FlacSpec, flac_constant, ogg_vorbis_silence};
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
            "yeban-mcp-sfzdecode-{}-{tag}-{}",
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

/// 写一份单区域 SFZ（`key=60` 与 `filled_project()` 的音符同音高），返回路径。
fn write_sfz(dir: &Path, name: &str, sample: &str) -> PathBuf {
    write_raw_sfz(
        dir,
        name,
        &format!("sample={sample} key=60 pitch_keycenter=60"),
    )
}

/// 写一份单区域 SFZ，region 体由调用方给（用于带多个 opcode 的判据夹具）。
fn write_raw_sfz(dir: &Path, name: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("建 SFZ 目录");
    let path = dir.join(name);
    std::fs::write(&path, format!("<region> {body}\n")).expect("写 SFZ");
    path
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-sfzdecode-{}.yeban", std::process::id())),
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

/// 一次装载 ＋ 一次渲染的公共读数：返回 `(load.data, render.data)`。
fn load_and_render(
    scratch: &Scratch,
    dispatcher: &mut Dispatcher,
    sfz: &Path,
    tag: &str,
) -> (Value, Value) {
    let loaded = call_tool(
        dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{tag}: {loaded}");
    let out = scratch.join(&format!("{tag}.wav"));
    let rendered = call_tool(
        dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": 1,
            "frames": 24_000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{tag}: {rendered}");
    (loaded["data"].clone(), rendered["data"].clone())
}

/// 判据 ①／③：**FLAC 真的被读**（修前这一条会红：`Ill-formed WAVE file` ⇒ 48/48 失败）。
///
/// 真实的 `freepats-spanish-classical-guitar` 是 48 个 `.flac` 区域；这里用
/// `testfix::flac_constant` 造一份**可解码**的 FLAC（512 帧 / 8 kHz）代替它，
/// 因为判据必须在不依赖用户采样库的机器上可跑。真实库的读数见报告里的扫描表。
#[test]
fn a_flac_sample_is_really_read_and_rendered() {
    let scratch = Scratch::new("flac");
    let dir = scratch.join("flac");
    std::fs::create_dir_all(&dir).expect("建目录");
    // 512 帧 @ 8 kHz、单声道、16-bit、常量 500 的合法 FLAC。
    std::fs::write(
        dir.join("tone.flac"),
        flac_constant(&FlacSpec::default(), 2, 500),
    )
    .expect("写 FLAC 夹具");
    let sfz = write_sfz(&dir, "tone.sfz", "tone.flac");

    let mut dispatcher = dispatcher_with_project();
    let (loaded, rendered) = load_and_render(&scratch, &mut dispatcher, &sfz, "flac");

    // ---- 装载：真的读到了 FLAC（⛔ 不是回退正弦）----
    assert_eq!(loaded["regions"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesRead"], true, "{loaded}");
    assert_eq!(loaded["sampleFilesReadCount"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesFailedCount"], 0, "{loaded}");
    assert_eq!(loaded["sampleFramesLoaded"], 512, "{loaded}");
    let source = loaded["sampleSource"].as_str().expect("sampleSource");
    assert!(source.starts_with("file("), "来源必须点名文件: {source}");
    assert!(
        source.contains("frames=512 rate=8000"),
        "FLAC 的帧数/采样率必须原样上报: {source}"
    );
    let state = loaded["regionDetails"][0]["sampleFileState"]
        .as_str()
        .expect("sampleFileState");
    assert!(state.starts_with("file("), "逐区域也要如实: {state}");

    // ---- 渲染：音符真的用了文件样本（⛔ 不是内存正弦回退）----
    assert!(
        rendered["notesUsingFileSample"].as_u64().unwrap_or(0) > 0,
        "必须真的有音符用了 FLAC 样本: {rendered}"
    );
    assert_eq!(rendered["notesUsingSineFallback"], 0, "{rendered}");
    assert_eq!(rendered["sampleFilesRead"], true, "{rendered}");
    assert!(
        rendered["sampleSources"].as_array().is_some_and(|list| list
            .iter()
            .any(|item| item.as_str().is_some_and(|text| text.contains("rate=8000")))),
        "渲染读数必须点名 FLAC 的采样率: {rendered}"
    );
    let rendered_source = rendered["sampleSource"].as_str().expect("sampleSource");
    assert!(
        rendered_source.starts_with("file("),
        "渲染读数必须点名读到的 FLAC: {rendered_source}"
    );
}

/// 判据 ②：**OGG 真的被读**（用 `yeban-decode` 的既有 OGG/Vorbis 夹具）。
#[test]
fn an_ogg_sample_is_really_read_and_rendered() {
    let scratch = Scratch::new("ogg");
    let dir = scratch.join("ogg");
    std::fs::create_dir_all(&dir).expect("建目录");
    // 1 声道、44.1 kHz、静音、128 帧的合法 Ogg Vorbis（`testfix` 的既有夹具）。
    std::fs::write(dir.join("tone.ogg"), ogg_vorbis_silence(4)).expect("写 OGG 夹具");
    let sfz = write_sfz(&dir, "tone.sfz", "tone.ogg");

    let mut dispatcher = dispatcher_with_project();
    let (loaded, rendered) = load_and_render(&scratch, &mut dispatcher, &sfz, "ogg");

    assert_eq!(loaded["regions"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesRead"], true, "{loaded}");
    assert_eq!(loaded["sampleFilesReadCount"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesFailedCount"], 0, "{loaded}");
    assert_eq!(loaded["sampleFramesLoaded"], 128, "{loaded}");
    let source = loaded["sampleSource"].as_str().expect("sampleSource");
    assert!(source.starts_with("file("), "来源必须点名文件: {source}");
    assert!(
        source.contains("frames=128 rate=44100"),
        "OGG 的帧数/采样率必须原样上报: {source}"
    );
    assert!(
        rendered["notesUsingFileSample"].as_u64().unwrap_or(0) > 0,
        "必须真的有音符用了 OGG 样本: {rendered}"
    );
    assert_eq!(rendered["notesUsingSineFallback"], 0, "{rendered}");
}

/// 判据 ③：**PCM WAV 不退化** —— 同一条语句在 `.wav` 上仍报逐字相同的读数。
///
/// `frames=48000 rate=48000` 这一串就是既有 WAV 腿的读数口径；本判据把它**逐字**钉住，
/// 因为 `#223` 改了 `read_sample_file` 的分派（WAV 必须仍走原腿）。
#[test]
fn a_plain_wav_keeps_its_exact_reading() {
    let scratch = Scratch::new("wav");
    let dir = scratch.join("wav");
    std::fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("tone.wav"), 0.5, 48_000, 48_000);
    let sfz = write_sfz(&dir, "tone.sfz", "tone.wav");

    let mut dispatcher = dispatcher_with_project();
    let (loaded, rendered) = load_and_render(&scratch, &mut dispatcher, &sfz, "wav");

    assert_eq!(loaded["regions"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesReadCount"], 1, "{loaded}");
    assert_eq!(loaded["sampleFilesFailedCount"], 0, "{loaded}");
    assert_eq!(loaded["sampleFramesLoaded"], 48_000, "{loaded}");
    let source = loaded["sampleSource"].as_str().expect("sampleSource");
    assert!(
        source.contains("frames=48000 rate=48000"),
        "WAV 腿的读数必须逐字不变: {source}"
    );
    assert!(
        rendered["notesUsingFileSample"].as_u64().unwrap_or(0) > 0,
        "{rendered}"
    );
    assert_eq!(rendered["notesUsingSineFallback"], 0, "{rendered}");
}

/// 判据 ② 的反向对照：**未知/无后缀**也走通用腿（与 app 的按后缀分派逐字同一条）。
///
/// 内容嗅探由 symphonia 按**字节**做 —— 文件名叫 `tone.bin`，字节是 FLAC ⇒ 读得到。
#[test]
fn a_flac_with_an_unknown_suffix_goes_through_the_generic_leg() {
    let scratch = Scratch::new("nosuffix");
    let dir = scratch.join("nosuffix");
    std::fs::create_dir_all(&dir).expect("建目录");
    std::fs::write(
        dir.join("tone.bin"),
        flac_constant(&FlacSpec::default(), 1, 500),
    )
    .expect("写无后缀 FLAC 夹具");
    let sfz = write_sfz(&dir, "tone.sfz", "tone.bin");

    let mut dispatcher = dispatcher_with_project();
    let (loaded, _) = load_and_render(&scratch, &mut dispatcher, &sfz, "nosuffix");
    assert_eq!(loaded["sampleFilesRead"], true, "{loaded}");
    assert_eq!(loaded["sampleFilesReadCount"], 1, "{loaded}");
    let source = loaded["sampleSource"].as_str().expect("sampleSource");
    assert!(
        source.contains("frames=256 rate=8000"),
        "无后缀也必须按内容解码: {source}"
    );
}

/// 判据 ④（采样器路径的另一半）：`yeban_render_sfz_wav` 也必须把那**四类**
/// 已知不生效的区域参数逐项登记进 `unsupported` —— ⛔ 不许"不生效也不报"。
///
/// 与 `yeban_render_master` 的 `the_known_unsupported_list_is_not_decoration` 互补：
/// 那个工具用 `KNOWN_UNSUPPORTED_KEYS` 的清单驱动，这个工具用自己的 `unsupported_keys()`。
/// 两边共用的判定函数在 `domain/sfz.rs`（`region_declares_*`），所以这条判据同时钉住
/// "两条采样器路径的分歧读数一致"。
#[test]
fn the_sfz_wav_tool_reports_the_four_known_gaps() {
    let scratch = Scratch::new("gaps");
    let dir = scratch.join("gaps");
    std::fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("gap.wav"), 0.5, 48_000, 48_000);
    let gap = write_raw_sfz(
        &dir,
        "gap.sfz",
        "sample=gap.wav key=60 pitch_keycenter=60 amplitude=50 \
         tune_oncc1=50 volume_oncc1=-6 offset=100 end=400",
    );

    let mut dispatcher = dispatcher_with_project();
    let (_, rendered) = load_and_render(&scratch, &mut dispatcher, &gap, "gaps");
    let unsupported: Vec<&str> = rendered["unsupported"]
        .as_array()
        .expect("unsupported")
        .iter()
        .map(|key| key.as_str().expect("字符串键"))
        .collect();
    for key in [
        "sfzAmplitude",
        "sfzTuneCcModulation",
        "sfzVolumeCcModulation",
        "sfzSampleSpan",
    ] {
        assert!(
            unsupported.contains(&key),
            "`{key}` 必须出现在 yeban_render_sfz_wav 的 unsupported 里: {unsupported:?}"
        );
    }

    // 控制臂：没有这四个 opcode 的同一份 SFZ ⇒ 一条都不许出现（⛔ 无条件登记）。
    let plain = write_sfz(&dir, "plain.sfz", "gap.wav");
    let mut dispatcher = dispatcher_with_project();
    let (_, plain_rendered) = load_and_render(&scratch, &mut dispatcher, &plain, "gaps-plain");
    let plain_unsupported: Vec<&str> = plain_rendered["unsupported"]
        .as_array()
        .expect("unsupported")
        .iter()
        .map(|key| key.as_str().expect("字符串键"))
        .collect();
    for key in [
        "sfzAmplitude",
        "sfzTuneCcModulation",
        "sfzVolumeCcModulation",
        "sfzSampleSpan",
    ] {
        assert!(
            !plain_unsupported.contains(&key),
            "`{key}` 不得在未声明的输入上出现: {plain_unsupported:?}"
        );
    }
}
