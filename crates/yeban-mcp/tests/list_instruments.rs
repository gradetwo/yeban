//! `yeban_list_instruments` 的承重判据（审计 `N13`）：会话 SFZ registry 的**唯一**
//! 枚举入口。两个读数判一件事——registry 里有什么，清单就逐字回什么。
//!
//! | 判据 | 量 | 单位 |
//! | :--- | :--- | :--- |
//! | 空 registry | `instruments` 长度 / `count` / `note` | 个 |
//! | 装载一件乐器后 | `instruments[0].instrumentId` 与 `yeban_load_sfz` 的响应逐字 | — |
//!
//! 夹具：单区域 SFZ ＋ 手写 24-bit PCM 常量 WAV（与 `sfz_decoded_formats.rs` 同口径，
//! ⛔ 不入库任何音频字节）。

use std::path::{Path, PathBuf};

use serde_json::Value;
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-listinst-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// **手写**一份常量值的 24-bit 单声道 WAV（与 `sfz_decoded_formats.rs` 同一份字节口径）。
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
    out.extend_from_slice(&1_u32.to_le_bytes()); // PCM / 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 3).to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
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
            std::env::temp_dir().join(format!("yeban-mcp-listinst-{}.yeban", std::process::id())),
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

/// 空 registry ⇒ `instruments: []` ＋ `count: 0` ＋ 指向 `yeban_load_sfz` 的 `note`。
#[test]
fn empty_registry_lists_zero_instruments() {
    let mut dispatcher = dispatcher_with_project();
    let listed = call_tool(
        &mut dispatcher,
        "yeban_list_instruments",
        serde_json::json!({}),
    );
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    assert_eq!(
        data["instruments"].as_array().map_or(usize::MAX, Vec::len),
        0
    );
    assert_eq!(data["count"], 0);
    assert_eq!(data["loadTool"], "yeban_load_sfz");
    assert!(data["note"].as_str().is_some_and(|n| !n.is_empty()));
}

/// 装载一件 ⇒ 清单逐字回显 `instrumentId` / `path` / `regions`，且 `count` 移到 1。
#[test]
fn loaded_instrument_appears_in_the_list() {
    let scratch = Scratch::new("loaded");
    let wav = scratch.dir.join("tone.wav");
    write_constant_wav(&wav, 0.25, 256, 48_000);
    let sfz = scratch.dir.join("one.sfz");
    std::fs::write(
        &sfz,
        format!(
            "<region> sample={} key=60 pitch_keycenter=60\n",
            wav.display()
        ),
    )
    .expect("写 SFZ 夹具");

    let mut dispatcher = dispatcher_with_project();
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let instrument_id = loaded["data"]["instrumentId"].clone();
    assert!(instrument_id.is_u64(), "{loaded}");

    let listed = call_tool(
        &mut dispatcher,
        "yeban_list_instruments",
        serde_json::json!({}),
    );
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    let instruments = data["instruments"].as_array().expect("instruments 是数组");
    assert_eq!(instruments.len(), 1, "{data}");
    assert_eq!(data["count"], 1);
    let entry = &instruments[0];
    assert_eq!(entry["instrumentId"], instrument_id);
    assert_eq!(entry["path"], sfz.display().to_string());
    assert_eq!(entry["regions"], 1);
    assert_eq!(entry["sessionScoped"], true);
}

// ---------------------------------------------------------------------------
// B7 目录模式：`directory` 给出时扫磁盘目录（⛔ 不查 registry、⛔ 不装载）。
// ---------------------------------------------------------------------------

fn call_list_directory(dispatcher: &mut Dispatcher, directory: &Path) -> Value {
    call_tool(
        dispatcher,
        "yeban_list_instruments",
        serde_json::json!({ "directory": directory.display().to_string() }),
    )
}

/// 会话模式（不给 `directory`）回 `mode: "session"`，与目录模式可区分。
#[test]
fn session_mode_reports_mode_session() {
    let mut dispatcher = dispatcher_with_project();
    let listed = call_tool(
        &mut dispatcher,
        "yeban_list_instruments",
        serde_json::json!({}),
    );
    assert_eq!(listed["status"], "success", "{listed}");
    assert_eq!(listed["data"]["mode"], "session", "{listed}");
}

/// 目录模式：递归列出 SFZ，逐件回 declared/present/missing 样本计数；
/// `*silence` 不计入声明数；汇总计数与逐件计数对得上。
#[test]
fn directory_lists_sfz_with_present_and_missing_samples() {
    let scratch = Scratch::new("dir-basic");
    let lib1 = scratch.dir.join("lib1");
    let lib2 = scratch.dir.join("lib2");
    std::fs::create_dir_all(&lib1).expect("建 lib1");
    std::fs::create_dir_all(&lib2).expect("建 lib2");
    let tone = lib1.join("tone.wav");
    write_constant_wav(&tone, 0.25, 256, 48_000);
    let one = lib1.join("one.sfz");
    std::fs::write(
        &one,
        format!(
            "<region> sample={} key=60 pitch_keycenter=60\n\
             <region> sample=missing.wav key=61 pitch_keycenter=61\n\
             <region> sample=*silence key=62 pitch_keycenter=62\n",
            tone.display()
        ),
    )
    .expect("写 one.sfz");
    let two_sample = lib2.join("hit.wav");
    write_constant_wav(&two_sample, 0.5, 128, 48_000);
    let two = lib2.join("two.sfz");
    std::fs::write(&two, "<region> sample=hit.wav key=60 pitch_keycenter=60\n")
        .expect("写 two.sfz");

    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &scratch.dir);
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    assert_eq!(data["mode"], "directory", "{data}");
    assert_eq!(data["count"], 2, "{data}");
    assert_eq!(data["declaredSampleCount"], 3, "{data}");
    assert_eq!(data["presentSampleCount"], 2, "{data}");
    assert_eq!(data["missingSampleCount"], 1, "{data}");
    assert_eq!(data["sampleFileCount"], 2, "{data}");
    assert_eq!(data["unsupportedFileCount"], 0, "{data}");
    assert_eq!(data["parseFailureCount"], 0, "{data}");
    assert_eq!(data["manifest"], Value::Null, "{data}");
    let instruments = data["instruments"].as_array().expect("instruments 是数组");
    assert_eq!(instruments.len(), 2, "{data}");
    // 路径升序：lib1/one.sfz 在 lib2/two.sfz 之前。
    let first = &instruments[0];
    assert_eq!(first["path"], one.display().to_string(), "{first}");
    assert_eq!(first["regions"], 3, "{first}");
    assert_eq!(first["declaredSampleCount"], 2, "{first}");
    assert_eq!(first["presentSampleCount"], 1, "{first}");
    assert_eq!(first["missingSampleCount"], 1, "{first}");
    assert_eq!(first["parseError"], Value::Null, "{first}");
    assert_eq!(first["sessionScoped"], false, "{first}");
    let examples = first["missingSampleExamples"]
        .as_array()
        .expect("missingSampleExamples 是数组");
    assert_eq!(examples.len(), 1, "{first}");
    assert!(
        examples[0]
            .as_str()
            .is_some_and(|p| p.ends_with("missing.wav")),
        "{first}"
    );
    let second = &instruments[1];
    assert_eq!(second["path"], two.display().to_string(), "{second}");
    assert_eq!(second["declaredSampleCount"], 1, "{second}");
    assert_eq!(second["presentSampleCount"], 1, "{second}");
    assert_eq!(second["missingSampleCount"], 0, "{second}");
}

/// 目录不存在 ⇒ 响亮 `IO_ERROR`（⛔ 不报成空清单）。
#[test]
fn directory_that_does_not_exist_is_a_loud_error() {
    let scratch = Scratch::new("dir-absent");
    let absent = scratch.dir.join("no-such-dir");
    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &absent);
    assert_eq!(listed["status"], "error", "{listed}");
    assert_eq!(listed["error"]["code"], "IO_ERROR", "{listed}");
    assert_eq!(
        listed["error"]["data"]["reason"], "listDirectoryNotFound",
        "{listed}"
    );
}

/// `directory` 给空串 ⇒ 响亮 `INVALID_PARAMETER_RANGE`（⛔ 不当作会话模式）。
#[test]
fn directory_empty_string_is_a_loud_error() {
    let mut dispatcher = dispatcher_with_project();
    let listed = call_tool(
        &mut dispatcher,
        "yeban_list_instruments",
        serde_json::json!({ "directory": "" }),
    );
    assert_eq!(listed["status"], "error", "{listed}");
    assert_eq!(
        listed["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{listed}"
    );
    assert_eq!(
        listed["error"]["data"]["reason"], "listDirectoryEmpty",
        "{listed}"
    );
}

/// `directory` 给的是普通文件 ⇒ 响亮 `IO_ERROR`。
#[test]
fn directory_pointing_at_a_file_is_a_loud_error() {
    let scratch = Scratch::new("dir-is-file");
    let file = scratch.dir.join("one.sfz");
    std::fs::write(&file, "<region> sample=*silence key=60\n").expect("写 SFZ 夹具");
    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &file);
    assert_eq!(listed["status"], "error", "{listed}");
    assert_eq!(listed["error"]["code"], "IO_ERROR", "{listed}");
    assert_eq!(
        listed["error"]["data"]["reason"], "listDirectoryNotDirectory",
        "{listed}"
    );
}

/// 空目录 ⇒ 成功回空清单 ＋ `note` 点名（没乐器不是错误，但必须说清）。
#[test]
fn empty_directory_lists_zero_with_a_note() {
    let scratch = Scratch::new("dir-empty");
    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &scratch.dir);
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    assert_eq!(data["count"], 0, "{data}");
    assert_eq!(
        data["instruments"].as_array().map_or(usize::MAX, Vec::len),
        0,
        "{data}"
    );
    assert_eq!(data["scannedFileCount"], 0, "{data}");
    assert!(
        data["note"].as_str().is_some_and(|n| !n.is_empty()),
        "{data}"
    );
}

/// 不支持的文件：⛔ 不进清单、⛔ 不报错，但计数并给例子点名。
#[test]
fn unsupported_files_are_counted_not_listed() {
    let scratch = Scratch::new("dir-unsupported");
    let sfz = scratch.dir.join("one.sfz");
    std::fs::write(&sfz, "<region> sample=*silence key=60 pitch_keycenter=60\n")
        .expect("写 SFZ 夹具");
    std::fs::write(scratch.dir.join("notes.txt"), "hello").expect("写 txt 夹具");
    std::fs::write(scratch.dir.join("data.xyz"), "??").expect("写 xyz 夹具");
    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &scratch.dir);
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    assert_eq!(data["count"], 1, "{data}");
    assert_eq!(data["unsupportedFileCount"], 2, "{data}");
    let examples = data["unsupportedExamples"]
        .as_array()
        .expect("unsupportedExamples 是数组");
    assert_eq!(examples.len(), 2, "{data}");
}

/// 坏 SFZ（非 UTF-8）仍占一条并带 `parseError` 点名，⛔ 不静默丢弃。
#[test]
fn unreadable_sfz_is_marked_not_dropped() {
    let scratch = Scratch::new("dir-bad-sfz");
    std::fs::write(scratch.dir.join("bad.sfz"), [0xff_u8, 0xfe, 0x00]).expect("写坏 SFZ");
    let good = scratch.dir.join("good.sfz");
    std::fs::write(
        &good,
        "<region> sample=*silence key=60 pitch_keycenter=60\n",
    )
    .expect("写好 SFZ");
    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &scratch.dir);
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    assert_eq!(data["count"], 2, "{data}");
    assert_eq!(data["parseFailureCount"], 1, "{data}");
    let instruments = data["instruments"].as_array().expect("instruments 是数组");
    let bad = &instruments[0];
    assert!(
        bad["path"].as_str().is_some_and(|p| p.ends_with("bad.sfz")),
        "{bad}"
    );
    assert!(bad["parseError"].is_string(), "{bad}");
    assert_eq!(bad["regions"], Value::Null, "{bad}");
    let good_entry = &instruments[1];
    assert_eq!(good_entry["parseError"], Value::Null, "{good_entry}");
    assert_eq!(good_entry["regions"], 1, "{good_entry}");
}

/// 根 `manifest.json`：逐文件 stat 回每件乐器的 present/missing 对账（B7 本体）。
#[test]
fn directory_manifest_reports_present_and_missing_per_instrument() {
    let scratch = Scratch::new("dir-manifest");
    let programs = scratch.dir.join("lib-a").join("Programs");
    let samples = scratch.dir.join("lib-a").join("Samples");
    std::fs::create_dir_all(&programs).expect("建 Programs");
    std::fs::create_dir_all(&samples).expect("建 Samples");
    std::fs::write(
        programs.join("a.sfz"),
        "<region> sample=../Samples/s1.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 a.sfz");
    write_constant_wav(&samples.join("s1.wav"), 0.25, 64, 48_000);
    // s2.wav 故意不写盘 ⇒ manifest 对账里算 missing。
    let manifest = serde_json::json!({
        "category": "samples",
        "instruments": [{
            "id": "lib-a",
            "name": "Lib A",
            "sfz": "Programs/a.sfz",
            "relative_root": "assets/samples/lib-a",
            "file_count": 3,
            "bundled": false
        }],
        "items": [
            {"instrument": "lib-a", "relative_path": "assets/samples/lib-a/Programs/a.sfz"},
            {"instrument": "lib-a", "relative_path": "assets/samples/lib-a/Samples/s1.wav"},
            {"instrument": "lib-a", "relative_path": "assets/samples/lib-a/Samples/s2.wav"}
        ]
    });
    std::fs::write(
        scratch.dir.join("manifest.json"),
        serde_json::to_string(&manifest).expect("序列化 manifest"),
    )
    .expect("写 manifest");

    let mut dispatcher = dispatcher_with_project();
    let listed = call_list_directory(&mut dispatcher, &scratch.dir);
    assert_eq!(listed["status"], "success", "{listed}");
    let data = &listed["data"];
    // SFZ 扫描面：a.sfz 一件，声明样本 s1 在盘上。
    assert_eq!(data["count"], 1, "{data}");
    assert_eq!(data["presentSampleCount"], 1, "{data}");
    assert_eq!(data["missingSampleCount"], 0, "{data}");
    // manifest.json 本身不算不支持文件。
    assert_eq!(data["unsupportedFileCount"], 0, "{data}");
    let report = &data["manifest"];
    assert_eq!(report["error"], Value::Null, "{report}");
    assert_eq!(report["totalFiles"], 3, "{report}");
    assert_eq!(report["presentFiles"], 2, "{report}");
    assert_eq!(report["missingFiles"], 1, "{report}");
    let rows = report["instruments"]
        .as_array()
        .expect("manifest instruments 是数组");
    assert_eq!(rows.len(), 1, "{report}");
    assert_eq!(rows[0]["id"], "lib-a", "{report}");
    assert_eq!(rows[0]["presentCount"], 2, "{report}");
    assert_eq!(rows[0]["missingCount"], 1, "{report}");
    assert_eq!(rows[0]["allPresent"], false, "{report}");
}
