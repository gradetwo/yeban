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
