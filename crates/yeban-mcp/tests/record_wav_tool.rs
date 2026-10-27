//! `yeban_record_wav` 的**承重判据**（`ARCH-REC-001` 三段接成 MCP 工具）。
//!
//! ## 覆盖的判据
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 真 MCP 调用 ⇒ WAV 落盘、可回读、**非静音**、帧数与"推入帧"一致 | `a_real_mcp_call_writes_a_readable_non_silent_take` |
//! | a' | **环满** ⇒ 丢弃被计数、⛔ 不 panic、帧对齐（写出 == 推入）、文件仍合法 | `a_full_ring_counts_drops_and_still_writes_a_valid_take` |
//! | b | 非法入参 ⇒ 结构化错误（既有码），⛔ 不 panic | `bad_arguments_are_structured_errors` |
//! | c | 不存在的工具名被拒（负对照） | `bad_arguments_are_structured_errors`（`yeban_nope`） |
//! | d | `dryRun` 一位都不改（不落盘）且与真做同一批字节 | `dry_run_writes_nothing_and_matches_the_real_call` |
//! | e | 交付形态：24-bit / 声道数 / 采样率与请求一致，`source` 如实说"内存正弦" | `a_real_mcp_call_writes_a_readable_non_silent_take` |
//! | f | 失败不留半成品（撞工程文件 / 撞锁文件） | `a_rejected_recording_leaves_no_file_behind` |
//! | g | [G12] 真做与预览都**明说**"样本不是工程音频"并指路 `yeban_render_master` | `the_take_is_signposted_as_not_the_project_audio` |
//!
//! ## 口径（如实标注）
//!
//! - 样本来源是**内存测试音**（440 Hz 正弦）⇒ `source` / `deviceStream` 必须如实说明，
//!   ⛔ 不假装碰了声卡；[G12] 且恒带 `notProjectBounce: true` 与
//!   `alternatives.projectBounce`（接工程源是待裁项 HD-69，不在本切片擅自接线）。
//! - `dropped` 是**帧**级（引擎的 `dropped_frames()` 是**样本**级；见
//!   `domain/record_wav.rs` 的模块文档 ②）。

use std::path::PathBuf;

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
            "yeban-mcp-rec-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn text(&self, name: &str) -> String {
        self.join(name).display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-rec-fixture-{}.yeban",
                std::process::id()
            )),
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
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

fn call_tool_raw(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> (u16, Option<i64>) {
    let auth = bearer(dispatcher);
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let status = outcome.http_status;
    let code = outcome.error_code();
    let response = outcome.response.expect("响应");
    assert!(response.result.is_some() || response.error.is_some());
    (status, code)
}

/// **独立**的极小 RIFF 解析器（⛔ 不用被测写入器自证）。
fn wav_read(bytes: &[u8]) -> (u16, u32, u16, Vec<i32>) {
    assert!(bytes.len() > 44, "WAV 必须比 44 字节头长");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut cursor = 12usize;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut samples: Vec<i32> = Vec::new();
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let size =
            u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().expect("4 字节")) as usize;
        let body = cursor + 8;
        match fourcc {
            b"fmt " => {
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().expect("2"));
                sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().expect("4"));
                bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().expect("2"));
            }
            b"data" => {
                assert_eq!(bits, 24, "位深恒为 24-bit");
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
    (channels, sample_rate, bits, samples)
}

#[test]
fn a_real_mcp_call_writes_a_readable_non_silent_take() {
    let scratch = Scratch::new("take");
    let mut dispatcher = dispatcher_with_project();
    let target = scratch.join("take.wav");
    let response = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": target.display().to_string(),
            "seconds": 0.25,
            "sampleRate": 48_000,
            "channels": 2,
        }),
    );
    assert_eq!(response["status"], "success", "{response}");
    let data = &response["data"];
    assert_eq!(data["recorded"], true);
    assert_eq!(data["sampleRate"], 48_000);
    assert_eq!(data["channels"], 2);
    assert_eq!(data["bitDepth"], 24);
    assert_eq!(data["requestedFrames"], 12_000, "0.25 秒 @48 kHz");
    assert_eq!(
        data["deviceStream"], false,
        "本工具⛔ 不碰声卡（样本是内存测试音）"
    );
    assert_eq!(data["source"], "in-memory-sine(440Hz)");
    // 默认环 = 1 秒 ⇒ 0.25 秒的录制不该丢弃，且写出帧 == 推入帧（整帧契约）。
    assert_eq!(data["dropped"], 0, "{data}");
    assert_eq!(data["framesWritten"], data["pushed"], "{data}");
    assert_eq!(data["framesWritten"], 12_000);

    // 文件存在、字节数一致、SHA-256 形状正确。
    let metadata = std::fs::metadata(&target).expect("WAV 必须存在");
    assert_eq!(data["bytes"].as_u64().expect("bytes"), metadata.len());
    assert_eq!(
        data["payloadBytes"].as_u64().expect("payload"),
        data["framesWritten"].as_u64().expect("frames") * 2 * 3,
        "载荷 = 帧 × 声道 × 3 字节"
    );
    let sha = data["sha256"].as_str().expect("sha256");
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));

    // 回读：格式与请求一致、非静音。
    let bytes = std::fs::read(&target).expect("读回");
    let (channels, sample_rate, bits, samples) = wav_read(&bytes);
    assert_eq!((channels, sample_rate, bits), (2, 48_000, 24));
    assert_eq!(samples.len(), 12_000 * 2, "回读样本数 = 帧 × 声道");
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    assert!(peak > 1_000, "录出的样本不许是静音（峰值 = {peak}）");
}

#[test]
fn a_full_ring_counts_drops_and_still_writes_a_valid_take() {
    let scratch = Scratch::new("full");
    let mut dispatcher = dispatcher_with_project();
    let target = scratch.join("overflow.wav");
    // 小环（100 帧）＋默认批大小（4096 帧/次）⇒ 生产期间**一次都不抽干**，
    // 于是 2400 帧的请求只有前 100 帧进得来 ⇒ 必然观察到帧级丢弃。
    // （⚠ 若把 `drainBatch` 也调小，交错会让环永远不满 —— 本判据第一版就是这样红的。）
    let response = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": target.display().to_string(),
            "seconds": 0.05,
            "capacityFrames": 100,
        }),
    );
    assert_eq!(response["status"], "success", "{response}");
    let data = &response["data"];
    let pushed = data["pushed"].as_u64().expect("pushed");
    let dropped = data["dropped"].as_u64().expect("dropped");
    let written = data["framesWritten"].as_u64().expect("framesWritten");
    assert!(dropped > 0, "小环必须观察到丢弃: {data}");
    assert_eq!(written, pushed, "整帧契约：写出帧 == 成功推入帧");
    assert_eq!(
        pushed + dropped,
        2_400,
        "框架不变量：推入 + 丢弃 == 尝试次数（0.05 秒 @48 kHz）"
    );
    // 即使丢了一半，文件仍必须合法可回读。
    let bytes = std::fs::read(&target).expect("读回");
    let (channels, sample_rate, bits, samples) = wav_read(&bytes);
    assert_eq!((channels, sample_rate, bits), (2, 48_000, 24));
    assert_eq!(samples.len() as u64, written * 2);
}

#[test]
fn bad_arguments_are_structured_errors() {
    let scratch = Scratch::new("bad");
    let mut dispatcher = dispatcher_with_project();

    // 未知工具名 ⇒ 被拒（负对照）。
    let (status, code) = call_tool_raw(&mut dispatcher, "yeban_nope", serde_json::json!({}));
    assert_eq!(status, 400);
    assert_eq!(code, Some(-32004));

    // 缺 path ⇒ 注册表层（既有管线）。
    let (status, code) = call_tool_raw(&mut dispatcher, "yeban_record_wav", serde_json::json!({}));
    assert_eq!(status, 400);
    assert_eq!(code, Some(-32602));

    for (label, arguments) in [
        ("emptyPath", serde_json::json!({ "path": "" })),
        (
            "zeroSeconds",
            serde_json::json!({ "path": scratch.text("a.wav"), "seconds": 0 }),
        ),
        (
            "negativeSeconds",
            serde_json::json!({ "path": scratch.text("b.wav"), "seconds": -1 }),
        ),
        (
            "secondsOverCap",
            serde_json::json!({ "path": scratch.text("c.wav"), "seconds": 100_000 }),
        ),
        (
            "badChannels",
            serde_json::json!({ "path": scratch.text("d.wav"), "channels": 3 }),
        ),
        (
            "zeroCapacity",
            serde_json::json!({ "path": scratch.text("e.wav"), "capacityFrames": 0 }),
        ),
        (
            "zeroDrainBatch",
            serde_json::json!({ "path": scratch.text("f.wav"), "drainBatch": 0 }),
        ),
        (
            "badSampleRate",
            serde_json::json!({ "path": scratch.text("g.wav"), "sampleRate": 0 }),
        ),
        (
            "badTone",
            serde_json::json!({ "path": scratch.text("h.wav"), "toneHz": -5 }),
        ),
    ] {
        let response = call_tool(&mut dispatcher, "yeban_record_wav", arguments);
        assert_eq!(response["status"], "error", "[{label}]: {response}");
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "[{label}] 必须用既有的参数码: {response}"
        );
    }
    for name in [
        "a.wav", "b.wav", "c.wav", "d.wav", "e.wav", "f.wav", "g.wav", "h.wav",
    ] {
        assert!(
            !scratch.join(name).exists(),
            "被拒绝的调用不得留下 `{name}`"
        );
    }
}

#[test]
fn dry_run_writes_nothing_and_matches_the_real_call() {
    let scratch = Scratch::new("dry");
    let mut dispatcher = dispatcher_with_project();
    let target = scratch.join("dry.wav");
    let dry = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": target.display().to_string(),
            "seconds": 0.1,
            "dryRun": true,
        }),
    );
    assert!(!target.exists(), "dryRun 不得写文件");
    let would = &dry["data"]["preview"]["wouldWrite"];
    assert_eq!(would["frames"], 4_800);
    let dry_sha = would["sha256"].as_str().expect("sha256").to_owned();
    let real = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": target.display().to_string(),
            "seconds": 0.1,
        }),
    );
    assert!(target.exists(), "真做必须落盘");
    assert_eq!(
        real["data"]["sha256"].as_str().expect("sha256"),
        dry_sha,
        "同一请求的 dryRun 与真做必须是同一批字节（无 bext/时间戳）"
    );
}

/// [G12] "record" 这个名字容易让人以为在录工程 ⇒ 真做响应与 `dryRun` 预览都必须
/// **显式**说清"样本不是工程音频"（`notProjectBounce: true`），并给出工程 bounce
/// 的替代路径（`alternatives.projectBounce.tool == "yeban_render_master"`）。
#[test]
fn the_take_is_signposted_as_not_the_project_audio() {
    let scratch = Scratch::new("signpost");
    let mut dispatcher = dispatcher_with_project();
    let real = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": scratch.join("sign.wav").display().to_string(),
            "seconds": 0.05,
        }),
    );
    assert_eq!(real["status"], "success", "{real}");
    let dry = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({
            "path": scratch.join("sign-dry.wav").display().to_string(),
            "seconds": 0.05,
            "dryRun": true,
        }),
    );
    assert_eq!(dry["status"], "success", "{dry}");
    for (label, data) in [
        ("真做响应", &real["data"]),
        ("dryRun 预览", &dry["data"]["preview"]),
    ] {
        assert_eq!(
            data["notProjectBounce"],
            serde_json::json!(true),
            "[{label}] 必须明说样本不是工程音频: {data}"
        );
        assert_eq!(
            data["alternatives"]["projectBounce"]["tool"], "yeban_render_master",
            "[{label}] 工程 bounce 的指路必须点名替代工具: {data}"
        );
        let why = data["alternatives"]["projectBounce"]["why"]
            .as_str()
            .unwrap_or_default();
        assert!(
            why.contains("yeban_record_wav") && why.contains("yeban_render_master"),
            "[{label}] 指路说明必须同时点名两个工具（调用方才接得上）: {why}"
        );
    }
}

#[test]
fn a_rejected_recording_leaves_no_file_behind() {
    let scratch = Scratch::new("atomic");
    let project_path = scratch.join("song.yeban");
    let mut dispatcher = Dispatcher::new(
        BearerToken::generate().token,
        ScopeSet::all(),
        RunMode::Production,
    );
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.clone(), filled_project(), false)
        .expect("注入工程");

    // 撞工程文件本身 ⇒ 拒绝。
    let clobber = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({ "path": project_path.display().to_string(), "seconds": 0.01 }),
    );
    assert_eq!(clobber["status"], "error", "{clobber}");
    assert_eq!(clobber["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert!(!project_path.exists());

    // 撞该工程的锁文件（`<工程文件名>.lock`）⇒ 拒绝。
    let lock_path = scratch.join("song.yeban.lock");
    let lock = call_tool(
        &mut dispatcher,
        "yeban_record_wav",
        serde_json::json!({ "path": lock_path.display().to_string(), "seconds": 0.01 }),
    );
    assert_eq!(lock["status"], "error", "{lock}");
    assert!(!lock_path.exists());
}

#[test]
fn the_tool_is_discoverable_and_declares_d25_codes() {
    let mut dispatcher = dispatcher_with_project();
    let auth = bearer(&dispatcher);
    let line = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), line);
    let listed = outcome
        .response
        .expect("响应")
        .result_value()
        .expect("result")["tools"]
        .as_array()
        .expect("tools")
        .clone();
    assert_eq!(listed.len(), yeban_mcp::tools::TOOL_COUNT);
    let descriptor = listed
        .iter()
        .find(|entry| entry["name"] == "yeban_record_wav")
        .expect("tools/list 里必须有 yeban_record_wav");
    let spec = yeban_mcp::tools::tool("yeban_record_wav").expect("注册表");
    assert_eq!(descriptor["annotations"]["specId"], spec.spec_id);
    assert!(descriptor["inputSchema"]["properties"]["dryRun"].is_object());
    for code in spec.errors {
        assert!(
            code.is_schema_contract(),
            "声明了 D25 联集之外的错误码 {code}"
        );
    }
}
