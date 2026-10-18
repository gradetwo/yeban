//! `yeban_load_sfz` 的 **`#include` 展开**承重判据（任务单 `#208`）。
//!
//! ## 为什么有这一条（缺口是实测的）
//!
//! `#206` 让 `SfzInstrument::load_with_includes` 真的展开 include，但 MCP 的
//! `domain/sfz.rs` 仍走 `parse_text`（纯文本模式，include 被忽略）⇒ 外部 agent 经
//! `yeban_load_sfz` 装载一份"满是 `#include`"的真库乐器时拿到 `regions = 0`。
//!
//! ## 覆盖的判据
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 两层 include（root → middle → leaf）经**真 MCP 调用**装载 ⇒ `regions > 0`，且样本文件**真被读** | `load_sfz_expands_includes_and_reports_region_and_sample_reads` |
//! | b | 缺失的 include **不让装载失败**，但逐条透出（行号/路径/原因） | 同上 ＋ `a_missing_include_is_reported_but_the_load_still_succeeds` |
//! | c | 循环 include（A↔B）⇒ 工具**返回错误**，⛔ 不挂死 | `a_cyclic_include_is_an_error_and_never_hangs` |
//! | d | 展开后的片段序列**被渲染路径复用**（被 include 的 region 真的能命中并出声） | `the_renderer_sees_the_included_regions` |
//!
//! ## 口径
//!
//! - 夹具是**临时目录**里的两层 SFZ ＋ 三个手写 24-bit WAV ⇒ 不联网、不依赖
//!   用户机器上装了任何样本库。
//! - "样本真被读"的读数是 `sampleFilesRead` / `sampleFilesReadCount` /
//!   `sampleFramesLoaded`（⛔ 不是 `energy`；本文件不断言内存正弦）。

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
            "yeban-mcp-sfzinc-{}-{tag}-{}",
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

/// 两层 include 的乐器目录树：
///
/// `instrument/root.sfz` → `parts/middle.sfz` → `parts/leaf.sfz`
/// （include 路径按 SFZ 惯例相对**基准目录**＝根 SFZ 所在目录）。
///
/// 根里另有一条**不存在**的 include（判据 b）。
fn nested_instrument(scratch: &Scratch) -> PathBuf {
    let root = scratch.join("instrument");
    let parts = root.join("parts");
    std::fs::create_dir_all(&parts).expect("建 parts");

    write_constant_wav(&root.join("leaf.wav"), 0.5, 480, 48_000);
    write_constant_wav(&root.join("middle.wav"), 0.25, 480, 48_000);
    write_constant_wav(&root.join("middle2.wav"), 0.125, 480, 48_000);

    std::fs::write(
        parts.join("leaf.sfz"),
        "<region> sample=leaf.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 leaf.sfz");
    std::fs::write(
        parts.join("middle.sfz"),
        "#include \"parts/leaf.sfz\"\n\
         <region> sample=middle.wav key=62 pitch_keycenter=62\n\
         <region> sample=middle2.wav key=64 pitch_keycenter=64\n",
    )
    .expect("写 middle.sfz");
    // ⚠ 根文件里**只有** include（没有 inline `<region>`）⇒ 修前 `regions` 必须是 **0**，
    //    否则"include 被忽略"这件事会被一条 inline region 掩盖。
    // 行号是判据的一部分：line 1 middle / line 2 missing。
    std::fs::write(
        root.join("root.sfz"),
        "#include \"parts/middle.sfz\"\n\
         #include \"parts/missing.sfz\"\n",
    )
    .expect("写 root.sfz");
    root.join("root.sfz")
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-sfzinc-{}.yeban", std::process::id())),
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

/// **独立**的极小 RIFF 解析器（i24 交错样本）—— 回读必须由另一份实现做。
fn wav_peak_i24(bytes: &[u8]) -> i32 {
    assert!(bytes.len() > 44, "WAV 必须至少有 44 字节头");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut cursor = 12usize;
    let mut peak = 0_i32;
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("4 字节长度"),
        ) as usize;
        let body = cursor + 8;
        if fourcc == b"data" {
            let end = (body + size).min(bytes.len());
            let mut at = body;
            while at + 3 <= end {
                let raw = i32::from_le_bytes([
                    bytes[at],
                    bytes[at + 1],
                    bytes[at + 2],
                    if bytes[at + 2] & 0x80 != 0 {
                        0xff
                    } else {
                        0x00
                    },
                ]);
                peak = peak.max(raw.abs());
                at += 3;
            }
            break;
        }
        cursor = body + size + (size % 2);
    }
    peak
}

// ---------------------------------------------------------------------------
// 判据 a ＋ b：两层 include 展开 ⇒ regions > 0 ＋ 样本真读 ＋ 缺失 include 不失败
// ---------------------------------------------------------------------------

#[test]
fn load_sfz_expands_includes_and_reports_region_and_sample_reads() {
    let scratch = Scratch::new("nested");
    let root = nested_instrument(&scratch);
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": root.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    // ⭐ 修前这里是 **0**（根文件只有 include；`parse_text` 把它们全部忽略）。
    //    两层都展开后 = leaf 1 + middle 2 = 3。
    assert_eq!(
        data["regions"], 3,
        "两层 include 的 3 条 region 必须全部展开: {data}"
    );

    // 样本文件**真被读**（三个 `sample=` 都指向存在的文件）。
    assert_eq!(data["sampleFilesRead"], true, "{data}");
    assert_eq!(data["sampleFilesReadCount"], 3, "{data}");
    assert_eq!(data["sampleFilesFailedCount"], 0, "{data}");
    assert_eq!(data["sampleFramesLoaded"], 3 * 480, "{data}");
    assert!(
        data["sampleSource"]
            .as_str()
            .expect("sampleSource")
            .starts_with("file("),
        "读到样本时来源必须点名文件: {data}"
    );
    // 每条 region 的 sampleFileState 都必须是 `file(...)`（⛔ 不是 unreadable）。
    let details = data["regionDetails"].as_array().expect("regionDetails");
    assert_eq!(details.len(), 3);
    for detail in details {
        assert!(
            detail["sampleFileState"]
                .as_str()
                .expect("sampleFileState")
                .starts_with("file("),
            "被 include 的 region 的样本也必须真读: {detail}"
        );
    }

    // 判据 b：缺失的 include 只记一条，⛔ 不让整份乐器失败。
    let issues = data["includeIssues"].as_array().expect("includeIssues");
    assert_eq!(issues.len(), 1, "恰好一条缺失 include: {data}");
    assert_eq!(issues[0]["line"], 2, "行号必须指回根文件: {data}");
    assert_eq!(issues[0]["path"], "parts/missing.sfz", "{data}");
    assert_eq!(issues[0]["kind"], "notFound", "{data}");
    assert_eq!(data["includeIssueCount"], 1, "{data}");
    println!(
        "[sfz-include] regions={} sampleFilesReadCount={} sampleFramesLoaded={} includeIssueCount={} issue={}",
        data["regions"],
        data["sampleFilesReadCount"],
        data["sampleFramesLoaded"],
        data["includeIssueCount"],
        issues[0]["display"],
    );
}

// ---------------------------------------------------------------------------
// 判据 b（单独）：缺失 include 不失败 ＋ 记录
// ---------------------------------------------------------------------------

#[test]
fn a_missing_include_is_reported_but_the_load_still_succeeds() {
    let scratch = Scratch::new("missing");
    let sfz = scratch.join("missing.sfz");
    std::fs::write(
        &sfz,
        "<region> sample=a.wav key=60\n#include \"ghost.sfz\"\n",
    )
    .expect("写夹具");
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    assert_eq!(data["regions"], 1, "inline region 照常解析: {data}");
    let issues = data["includeIssues"].as_array().expect("includeIssues");
    assert_eq!(issues.len(), 1, "{data}");
    assert_eq!(issues[0]["line"], 2, "{data}");
    assert_eq!(issues[0]["path"], "ghost.sfz", "{data}");
    assert_eq!(issues[0]["kind"], "notFound", "{data}");

    // `dryRun` 预览也必须透出同一份 include 读数（⛔ 不是只有真做才报）。
    let preview = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string(), "dryRun": true }),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    assert_eq!(
        preview["data"]["preview"]["includeIssues"][0]["path"], "ghost.sfz",
        "preview 必须透出 includeIssues: {preview}"
    );
    assert_eq!(
        preview["data"]["preview"]["includeIssueCount"], 1,
        "{preview}"
    );
}

// ---------------------------------------------------------------------------
// 判据 c：循环 include ⇒ 错误，⛔ 不挂死
// ---------------------------------------------------------------------------

#[test]
fn a_cyclic_include_is_an_error_and_never_hangs() {
    let scratch = Scratch::new("cycle");
    std::fs::write(scratch.join("a.sfz"), "#include \"b.sfz\"\n").expect("写 a.sfz");
    std::fs::write(scratch.join("b.sfz"), "#include \"a.sfz\"\n").expect("写 b.sfz");
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": scratch.join("a.sfz").display().to_string() }),
    );
    assert_eq!(
        loaded["status"], "error",
        "循环 include 必须是错误: {loaded}"
    );
    assert_eq!(
        loaded["error"]["code"], "INVALID_PARAMETER_RANGE",
        "既有码联集里没有 SFZ 专用码: {loaded}"
    );
    assert_eq!(
        loaded["error"]["data"]["reason"], "sfzParseFailed",
        "{loaded}"
    );
    let message = loaded["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("include cycle"),
        "错误文案必须点名循环: {message}"
    );
    println!(
        "[sfz-include] cycle: code={} reason={} message={message}",
        loaded["error"]["code"], loaded["error"]["data"]["reason"],
    );
}

// ---------------------------------------------------------------------------
// 判据 d：展开后的片段序列被渲染路径复用
// ---------------------------------------------------------------------------

#[test]
fn the_renderer_sees_the_included_regions() {
    let scratch = Scratch::new("render");
    let root = nested_instrument(&scratch);
    let mut dispatcher = dispatcher_with_project();

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": root.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let instrument_id = loaded["data"]["instrumentId"]
        .as_u64()
        .expect("instrumentId");

    // 60 只由**被 include 的** `parts/leaf.sfz` 声明（根文件里没有 region）⇒
    // 渲染器必须看到展开后的片段，否则 `notes_unmatched` 会静音。
    let target = scratch.join("rendered.wav");
    let rendered = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 4800,
            "note": 60,
            "path": target.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let bytes = std::fs::read(&target).expect("读回渲染 WAV");
    let peak = wav_peak_i24(&bytes);
    assert!(
        peak > 1_000_000,
        "命中被 include 的 region 且样本 0.5 ⇒ 必须明显非静音（⛔ 不是抖动地板）, 实得 peak={peak}"
    );
    println!(
        "[sfz-include] render note=60 peak={peak} bytes={}",
        bytes.len()
    );
}

// ---------------------------------------------------------------------------
// 可选：仓库里**已登记**的真库乐器（不存在就跳过）
// ---------------------------------------------------------------------------

/// 真库乐器（`assets/samples/**` 里的 `.sfz`）经**真 MCP 调用**装载 ⇒ `regions > 0`。
///
/// ⚠ 这是**可选**判据：`assets/samples/**` 只随仓库分发，若某台机器上没有这份夹具，
/// 本用例打印 `skip` 并返回（上一条临时夹具用例仍然承重）。它**不联网**、也**不**依赖
/// 用户机器上装了任何样本库。
#[test]
fn a_checked_in_library_instrument_loads_through_its_includes() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let cases = [
        (
            "salamander-grand",
            repo.join("assets/samples/salamander-grand/Salamander Grand Piano V3.sfz"),
        ),
        (
            "body-percussion",
            repo.join("assets/samples/body-percussion/Programs/body.sfz"),
        ),
    ];
    let mut dispatcher = dispatcher_with_project();
    let mut ran = 0_u32;
    for (name, path) in cases {
        if !path.is_file() {
            println!(
                "[sfz-include] skip real library {name}: {} absent",
                path.display()
            );
            continue;
        }
        let loaded = call_tool(
            &mut dispatcher,
            "yeban_load_sfz",
            serde_json::json!({ "path": path.display().to_string() }),
        );
        assert_eq!(loaded["status"], "success", "{name}: {loaded}");
        let data = &loaded["data"];
        assert!(
            data["regions"].as_u64().unwrap_or(0) > 0,
            "{name}: include 展开后必须 > 0 个 region: {data}"
        );
        println!(
            "[sfz-include] real-library {name}: regions={} sampleFilesReadCount={} sampleFilesFailedCount={} includeIssueCount={}",
            data["regions"],
            data["sampleFilesReadCount"],
            data["sampleFilesFailedCount"],
            data["includeIssueCount"],
        );
        ran += 1;
    }
    if ran == 0 {
        println!(
            "[sfz-include] no checked-in library fixture on this machine ⇒ skipped (temporary fixtures above still cover the criterion)"
        );
    }
}
