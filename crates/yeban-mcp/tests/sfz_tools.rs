//! `yeban_load_sfz` / `yeban_render_sfz_wav` 的**承重判据**（本线的两条新工具）。
//!
//! ## 覆盖的判据（逐条对应任务书）
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 新工具经 MCP 调用**真的产出结果**：WAV 落盘、字节数/摘要可判定、**非静音**（回读并算峰值/能量） | `render_sfz_wav_writes_a_non_silent_wav_through_a_real_mcp_call` |
//! | a' | SFZ 工具**返回 region 数与来源**，且区域数与文件内容一致（反向对照：单区域文件 ⇒ 1） | `load_sfz_reports_the_region_count_and_the_honest_source` |
//! | a'' | `sampleHz` 入参**真的被读**：非缺省值进报告、非正数被领域层拒、非数字被注册表层拒（"必须是数字"支经 MCP 不可达，由 src 单元测试钉住） | `the_sample_hz_argument_is_honoured_and_bad_values_are_rejected` |
//! | b | 非法入参 ⇒ **结构化**错误（既有错误码）、⛔ 不 panic | `bad_arguments_are_structured_errors_and_never_panic` |
//! | c | 不存在/写错的工具名 ⇒ 被拒（`yeban_nope` 负对照） | `an_unknown_tool_name_is_rejected` |
//! | d | `dryRun` 一位都不改；预览与真做的实测数字一致 | `dry_run_writes_nothing_and_matches_the_real_call` |
//! | e | 失败**不留半成品**（坏 `frames` / 撞工程路径） | `a_rejected_render_leaves_no_file_behind` |
//! | f | 会话态语义：打开另一个工程清空 registry；内容寻址复用同一个 id | `the_registry_is_session_scoped_and_content_addressed` |
//! | g | 契约一致性：新工具在 `tools/list` 里、错误码全在 D25 联集内 | `the_two_new_tools_are_discoverable_and_use_d25_codes` |
//!
//! ## 口径（如实标注）
//!
//! - 采样文件**会被真读**（`sample=` 指向的 PCM WAV；读不到则如实回退内存正弦）⇒ 响应
//!   `sampleSource` / `sampleFilesRead` 必须如实说明；判据断言这两件事，⛔ 不假装读到了 `.wav`。
//! - 本文件的运行期判据需要编译 `yeban-mcp`（传递依赖 rayon/symphonia/rubato/hound）。

use std::path::PathBuf;

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools;
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个临时目录，`Drop` 时清理。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-sfz-{}-{tag}-{}",
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

    /// 写一份 SFZ 并返回它的路径。
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

/// 一个新的分发器（内存工程，不碰仓库）。
fn dispatcher_with_project(project: yeban_model::YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-sfz-fixture-{}.yeban",
                std::process::id()
            )),
            project,
            false,
        )
        .expect("注入规范工程");
    dispatcher
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `result`（要求**没有** JSON-RPC 层错误）。
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

/// 一次 `tools/call` 的 `(http 状态, JSON-RPC 错误码)`。
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
    assert!(
        response.result.is_some() || response.error.is_some(),
        "响应必须要么带 result 要么带 error"
    );
    (status, code)
}

/// 两区域最小 SFZ（键范围不同）。
const TWO_REGIONS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                           <region> sample=b.wav key=64 pitch_keycenter=64\n";

/// 从 WAV 字节里读出 `data` 载荷（**独立**的极小 RIFF 解析器，⛔ 不用被测写入器自证）。
///
/// 只认 RIFF/WAVE + `fmt ` 24-bit + `data`：判据要的"回读"必须由**另一份实现**做。
fn wav_payload_i24(bytes: &[u8]) -> (u16, u32, u16, Vec<i32>) {
    assert!(bytes.len() > 44, "WAV 必须至少有 44 字节头");
    assert_eq!(&bytes[0..4], b"RIFF", "必须是 RIFF");
    assert_eq!(&bytes[8..12], b"WAVE", "必须是 WAVE");
    let mut cursor = 12usize;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut samples: Vec<i32> = Vec::new();
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("4 字节长度"),
        ) as usize;
        let body = cursor + 8;
        match fourcc {
            b"fmt " => {
                // `fmt ` 载荷布局：audioFormat@0, numChannels@2, sampleRate@4, …,
                // blockAlign@12, bitsPerSample@14。
                channels =
                    u16::from_le_bytes(bytes[body + 2..body + 4].try_into().expect("2 字节声道数"));
                sample_rate =
                    u32::from_le_bytes(bytes[body + 4..body + 8].try_into().expect("4 字节采样率"));
                bits =
                    u16::from_le_bytes(bytes[body + 14..body + 16].try_into().expect("2 字节位深"));
            }
            b"data" => {
                assert_eq!(bits, 24, "本工具的输出位深恒为 24-bit");
                let end = (body + size).min(bytes.len());
                let mut at = body;
                while at + 3 <= end {
                    let raw = i32::from_le_bytes([
                        bytes[at],
                        bytes[at + 1],
                        bytes[at + 2],
                        // 24-bit 有符号：最高字节按符号扩展（与 `hound` 的回读口径一致）。
                        if bytes[at + 2] & 0x80 != 0 {
                            0xff
                        } else {
                            0x00
                        },
                    ]);
                    samples.push(raw);
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

// ---------------------------------------------------------------------------
// 判据 a：真 MCP 调用 ⇒ 真的写出一份**非静音**的 WAV
// ---------------------------------------------------------------------------

#[test]
fn render_sfz_wav_writes_a_non_silent_wav_through_a_real_mcp_call() {
    let scratch = Scratch::new("render");
    let sfz = scratch.write_sfz("two-regions.sfz", TWO_REGIONS);
    let mut dispatcher = dispatcher_with_project(filled_project());

    // ---- ① 装载 ----
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let data = &loaded["data"];
    assert_eq!(data["loaded"], true);
    assert_eq!(data["regions"], 2, "两条 `<region>` ⇒ 区域数 2: {data}");
    // ⚠ 本夹具的 `sample=a.wav` / `b.wav` **不存在** ⇒ 如实回退：来源文案带**回退原因**，
    // 而 `sampleFilesRead` 是 `false`（⛔ 不假装读到）。真读到的情形见
    // `tests/sfz_sample_files.rs`。
    assert!(
        data["sampleSource"]
            .as_str()
            .expect("sampleSource")
            .starts_with("in-memory-sine"),
        "读不到 ⇒ 必须回退到内存正弦: {data}"
    );
    assert_eq!(data["sampleFilesRead"], false, "读不到就不许说读到了");
    assert_eq!(data["sampleFilesReadCount"], 0);
    assert!(
        data["sampleFilesFailedCount"].as_u64().unwrap_or(0) > 0,
        "读不到的文件必须被如实记进 sampleFailures: {data}"
    );
    assert!(
        data["sampleFailures"][0]
            .as_str()
            .expect("sampleFailures[0]")
            .starts_with("unreadable("),
        "失败项必须点名路径与原因: {data}"
    );
    // ⭐ G4：样本**全部**读不到 ⇒ `data.warnings[]` 必须点名 `sfzSamplesUnreadable`
    // —— "文本解析成功"不许被读成"有声音"（采样器轨没有正弦回退，绑轨即整轨静音）。
    let warnings = data["warnings"].as_array().expect("warnings 数组");
    assert_eq!(warnings.len(), 1, "全部缺样本 ⇒ 恰好一条警告: {data}");
    assert_eq!(warnings[0]["code"], "sfzSamplesUnreadable");
    assert_eq!(warnings[0]["severity"], "warning");
    assert_eq!(warnings[0]["blocking"], false, "装载本身不失败, 只点名");
    assert_eq!(warnings[0]["allSamplesUnreadable"], true);
    assert_eq!(
        warnings[0]["count"].as_u64(),
        data["sampleFilesFailedCount"].as_u64(),
        "警告计数与 `sampleFilesFailedCount` 必须是同一读数"
    );
    assert!(
        warnings[0]["examples"].as_array().expect("examples").len() <= 5,
        "例子最多 5 条: {data}"
    );
    assert!(
        warnings[0]["hint"]
            .as_str()
            .expect("hint")
            .contains("assets/samples/manifest.json"),
        "hint 必须点名登记式外取的清单位置: {data}"
    );
    assert_eq!(data["sessionScoped"], true);
    let instrument_id = data["instrumentId"].as_u64().expect("instrumentId 是整数");
    assert!(instrument_id >= 1);
    assert!(data["sha256"].as_str().expect("sha256").len() == 64);
    // 逐区域摘要必须点名样本路径（键范围来自解析器，不是我们编的）。
    let details = data["regionDetails"].as_array().expect("区域摘要");
    assert_eq!(details.len(), 2);
    assert_eq!(details[0]["sample"], "a.wav");
    assert_eq!(details[1]["sample"], "b.wav");
    assert_eq!(details[0]["keyRange"][0], 60);
    assert_eq!(details[1]["keyRange"][0], 64);
    assert_eq!(data["contentAddressed"], true);

    // ---- ② 导出：真的落盘 ----
    let target = scratch.join("sfz-out.wav");
    let rendered = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 24_000,
            "path": target.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let out = &rendered["data"];
    assert_eq!(out["rendered"], true);
    assert_eq!(out["frames"], 24_000);
    assert_eq!(out["sampleRate"], 48_000);
    assert_eq!(out["bitDepth"], 24);
    assert_eq!(out["channels"], 2);
    assert_eq!(
        out["sampleFilesRead"], false,
        "夹具的 sample= 文件不存在 ⇒ 回退正弦"
    );
    assert!(
        out["notesUsingSineFallback"].as_u64().unwrap_or(0) > 0,
        "回退必须被计数: {out}"
    );
    assert!(
        out["notesScheduled"].as_u64().unwrap_or(0) > 0,
        "夹具工程必须有 MIDI 音符被排程: {out}"
    );

    // ---- ③ 文件存在、字节数可判定、sha256 是 64 位十六进制 ----
    let metadata = std::fs::metadata(&target).expect("WAV 必须存在");
    assert!(metadata.len() > 44, "WAV 必须比 RIFF 头长");
    assert_eq!(
        out["bytes"].as_u64().expect("bytes"),
        metadata.len(),
        "响应里的字节数必须等于磁盘上的字节数"
    );
    let sha = out["sha256"].as_str().expect("sha256");
    assert_eq!(sha.len(), 64, "SHA-256 必须是 64 位十六进制");
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));

    // ---- ④ 非静音（用**独立**的 RIFF 回读 + 峰值/能量）----
    let bytes = std::fs::read(&target).expect("读回 WAV");
    let (channels, sample_rate, bits, samples) = wav_payload_i24(&bytes);
    assert_eq!(channels, 2);
    assert_eq!(sample_rate, 48_000);
    assert_eq!(bits, 24);
    assert_eq!(
        samples.len(),
        24_000 * 2,
        "回读样本数必须等于 帧数 × 声道数"
    );
    let peak = samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    let energy: i64 = samples.iter().map(|s| i64::from(*s)).map(|s| s * s).sum();
    assert!(
        peak > 1_000,
        "导出的 WAV 不许是静音（峰值 = {peak}；抖动地板只有 1 LSB）"
    );
    assert!(energy > 0, "能量必须大于 0（实得 {energy}）");
    // ⭐ 反向对照：**不命中**的音高（工程里没有的音符）⇒ 静音。
    //    "有声音"与"任何输入都有声音"是两件事 —— 后者会让上一条断言失去信息量。
    let silent = scratch.join("silent.wav");
    let silent_render = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": silent.display().to_string(),
            "note": 1,
        }),
    );
    assert_eq!(silent_render["status"], "success", "{silent_render}");
    assert_eq!(
        silent_render["data"]["notesScheduled"], 0,
        "`note: 1` 在夹具里没有对应音符 ⇒ 排程数必须是 0"
    );
    let silent_bytes = std::fs::read(&silent).expect("读回对照 WAV");
    let (_, _, _, silent_samples) = wav_payload_i24(&silent_bytes);
    let silent_peak = silent_samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    // ⚠ 口径：TPDF 抖动让"全零信号"落在 ±1 LSB 上 —— 因此**静音的读数是 ≤ 1**，
    // 不是 0（写成 `== 0` 会把"抖动在工作"误判成失败；这是本判据第一版的红）。
    // 有牙的判别式是两档的**量级差**：真渲染 ≫ 抖动地板。
    assert!(
        silent_peak <= 1,
        "没有音符命中 ⇒ 只应有抖动地板 (≤ 1 LSB), 实得峰值 {silent_peak}"
    );
}

// ---------------------------------------------------------------------------
// 判据 a'：region 数与来源如实（含反向对照）
// ---------------------------------------------------------------------------

#[test]
fn load_sfz_reports_the_region_count_and_the_honest_source() {
    let scratch = Scratch::new("load");
    let one = scratch.write_sfz("one.sfz", "<region> sample=only.wav key=60\n");
    let two = scratch.write_sfz("two.sfz", TWO_REGIONS);
    let mut dispatcher = dispatcher_with_project(filled_project());

    let single = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": one.display().to_string() }),
    );
    assert_eq!(single["data"]["regions"], 1, "单区域文件 ⇒ 1: {single}");
    assert_eq!(
        single["data"]["bytes"].as_u64().expect("bytes"),
        std::fs::metadata(&one).expect("文件").len(),
        "响应里的字节数必须是实测值"
    );

    let double = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": two.display().to_string() }),
    );
    assert_eq!(double["data"]["regions"], 2, "两区域文件 ⇒ 2: {double}");
    assert_eq!(
        double["data"]["sampleHz"], 48_000.0,
        "sampleHz 缺省必须是 48000"
    );
    // 两份不同的内容 ⇒ 两个不同的 instrumentId。
    assert_ne!(
        single["data"]["instrumentId"], double["data"]["instrumentId"],
        "不同内容必须是不同的 instrumentId"
    );
}

// ---------------------------------------------------------------------------
// 判据 a''：`sampleHz` 入参真的被读（非缺省回显 ＋ 两条拒绝路径）
// ---------------------------------------------------------------------------

/// 这个键此前**只有输出被断言**（上面那条 `sampleHz 缺省必须是 48000` 读的是响应），
/// 输入侧从未被任何一个判据驱动过 ⇒ 把它改成常量、或把 `parse_load_arguments` 的
/// 校验整段删掉，旧判据都不会红。本判据补的就是输入侧：
///
/// 1. 非缺省值 ⇒ 响应回显它（内容寻址的 `instrumentId` 不变 —— 身份只按内容算）；
/// 2. 非正数（0 / 负数）⇒ 领域层 `INVALID_PARAMETER_RANGE`，文案点名"必须是有限正数"；
/// 3. 非数字 ⇒ **注册表**层就拒（`-32602`）：`ParamSpec` 的 `number` 类型先于领域层，
///    因此 `parse_load_arguments` 的"必须是数字"支经 `tools/call` 不可达 ——
///    那一支由 `src/domain/sfz.rs` 的单元测试
///    `load_arguments_take_a_default_sample_hz_and_reject_bad_values` 直接钉住。
///
/// ⚠ 如实标注：本仓当前**没有任何下游消费者**读 `SfzLoad::sample_hz`（`src` 里唯一
/// 出现点是序列化那一行）⇒ 这个键今天的作用就是"进报告"。判据因此只断言回显，
/// ⛔ 假装它改变了渲染。（schema 描述里的"播放速率比"是描述而非实现读数。）
#[test]
fn the_sample_hz_argument_is_honoured_and_bad_values_are_rejected() {
    let scratch = Scratch::new("sample-hz");
    let sfz = scratch.write_sfz("one.sfz", "<region> sample=only.wav key=60\n");
    let mut dispatcher = dispatcher_with_project(filled_project());

    let at_441 = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string(), "sampleHz": 44_100 }),
    );
    assert_eq!(
        at_441["data"]["sampleHz"], 44_100.0,
        "非缺省的 sampleHz 必须被读并回显: {at_441}"
    );
    assert_eq!(at_441["data"]["regions"], 1, "这个键不影响解析: {at_441}");

    // 同一份内容、缺省 sampleHz ⇒ 同一身份、不同报告值。
    let defaulted = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(
        defaulted["data"]["sampleHz"], 48_000.0,
        "缺省必须是 48000: {defaulted}"
    );
    assert_eq!(
        at_441["data"]["instrumentId"], defaulted["data"]["instrumentId"],
        "身份按内容寻址 ⇒ sampleHz 不得影响 instrumentId"
    );
    assert_eq!(
        defaulted["data"]["reused"].as_bool(),
        Some(true),
        "第二次装载同一份内容必须标 reused: {defaulted}"
    );

    // ---- 非正数 ⇒ 领域层拒绝 ----
    for bad in [0, -1] {
        let rejected = call_tool(
            &mut dispatcher,
            "yeban_load_sfz",
            serde_json::json!({ "path": sfz.display().to_string(), "sampleHz": bad }),
        );
        assert_eq!(rejected["status"], "error", "{rejected}");
        assert_eq!(
            rejected["error"]["code"], "INVALID_PARAMETER_RANGE",
            "{rejected}"
        );
        let message = rejected["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("必须是有限正数 (Hz)"),
            "文案必须点名要求: {message}"
        );
        assert!(
            message.contains(&format!("实际收到 {bad}")),
            "文案必须点名实际收到的取值: {message}"
        );
    }

    // ---- 非数字 ⇒ 注册表层拒（-32602），⛔ 不进领域层 ----
    let (status, code) = call_tool_raw(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string(), "sampleHz": "fast" }),
    );
    assert_eq!(status, 400, "非法类型必须是 JSON-RPC 层拒绝");
    assert_eq!(code, Some(-32602), "非法类型 ⇒ -32602 (invalid params)");
}

// ---------------------------------------------------------------------------
// 判据 b：非法入参 ⇒ 结构化错误（既有码），⛔ 不 panic
// ---------------------------------------------------------------------------

#[test]
fn bad_arguments_are_structured_errors_and_never_panic() {
    let scratch = Scratch::new("bad");
    let sfz = scratch.write_sfz("two-regions.sfz", TWO_REGIONS);
    let mut dispatcher = dispatcher_with_project(filled_project());

    // ---- 未知工具名 ⇒ 被拒（判据 c 的一半；`yeban_nope` 负对照）----
    let (status, code) = call_tool_raw(&mut dispatcher, "yeban_nope", serde_json::json!({}));
    assert_eq!(status, 400, "未知工具名必须是 JSON-RPC 层拒绝");
    assert_eq!(
        code,
        Some(-32004),
        "未知工具名 ⇒ -32004 (Method not found 族)"
    );

    // ---- 缺 path：**注册表**层就拦住（既有管线：必填缺失 ⇒ JSON-RPC -32602）----
    // 这条与领域层无关 —— 判定它走的是 `ToolSpec::validate_arguments`，与所有工具同一条口径。
    let (status, code) = call_tool_raw(&mut dispatcher, "yeban_load_sfz", serde_json::json!({}));
    assert_eq!(status, 400, "缺必填参数必须是 JSON-RPC 层拒绝");
    assert_eq!(code, Some(-32602), "缺必填参数 ⇒ -32602 (invalid params)");

    // ---- 空 path ----
    let empty = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": "" }),
    );
    assert_eq!(empty["status"], "error");
    assert_eq!(empty["error"]["code"], "INVALID_PARAMETER_RANGE");

    // ---- 路径不存在 ⇒ IO_ERROR（联集里的既有码，⛔ 不发明 SFZ 专用码）----
    let absent = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": scratch.text("nope.sfz") }),
    );
    assert_eq!(absent["status"], "error", "{absent}");
    assert_eq!(absent["error"]["code"], "IO_ERROR");

    // ---- 路径是**目录** ⇒ IO_ERROR ----
    let as_dir = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": scratch.dir.display().to_string() }),
    );
    assert_eq!(as_dir["status"], "error", "{as_dir}");
    assert_eq!(as_dir["error"]["code"], "IO_ERROR");

    // ---- 非法 UTF-8 ⇒ INVALID_PARAMETER_RANGE（⛔ 不用替换字符"发明输入"）----
    let binary = scratch.join("binary.sfz");
    std::fs::write(&binary, [0x3c, 0x72, 0x3e, 0xff, 0xfe]).expect("写二进制夹具");
    let not_utf8 = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": binary.display().to_string() }),
    );
    assert_eq!(not_utf8["status"], "error", "{not_utf8}");
    assert_eq!(not_utf8["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(not_utf8["error"]["data"]["reason"], "sfzNotUtf8");

    // ---- 超限 SFZ（单行超长会被解析器的 limits 拒绝）----
    let huge_line = format!("<region> sample={}.wav key=60\n", "x".repeat(200_000));
    let over = scratch.write_sfz("over.sfz", &huge_line);
    let too_big = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": over.display().to_string() }),
    );
    assert_eq!(too_big["status"], "error", "{too_big}");
    assert_eq!(too_big["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(too_big["error"]["data"]["reason"], "sfzParseFailed");

    // ---- 装载成功之后，导出侧的坏参数 ----
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    let instrument_id = loaded["data"]["instrumentId"].as_u64().expect("id");

    for (label, arguments) in [
        // 未知 instrumentId ⇒ 既有码 + reason
        (
            "unknownInstrument",
            serde_json::json!({ "instrumentId": 9999, "frames": 128, "path": scratch.text("a.wav") }),
        ),
        // instrumentId = 0 不是合法身份
        (
            "zeroInstrument",
            serde_json::json!({ "instrumentId": 0, "frames": 128, "path": scratch.text("b.wav") }),
        ),
        // frames 必须 > 0
        (
            "zeroFrames",
            serde_json::json!({ "instrumentId": instrument_id, "frames": 0, "path": scratch.text("c.wav") }),
        ),
        // frames 超出单次上限
        (
            "framesOverCap",
            serde_json::json!({
                "instrumentId": instrument_id,
                "frames": 172_800_001_u64,
                "path": scratch.text("d.wav"),
            }),
        ),
        // velocity 越界
        (
            "velocityOverRange",
            serde_json::json!({
                "instrumentId": instrument_id, "frames": 128, "velocity": 128,
                "path": scratch.text("e.wav"),
            }),
        ),
        // note 越界
        (
            "noteOverRange",
            serde_json::json!({
                "instrumentId": instrument_id, "frames": 128, "note": 200,
                "path": scratch.text("f.wav"),
            }),
        ),
        // 空输出路径
        (
            "emptyPath",
            serde_json::json!({ "instrumentId": instrument_id, "frames": 128, "path": "" }),
        ),
    ] {
        let response = call_tool(&mut dispatcher, "yeban_render_sfz_wav", arguments);
        assert_eq!(
            response["status"], "error",
            "[{label}] 必须是带内错误: {response}"
        );
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "[{label}] 必须用既有的参数码: {response}"
        );
    }

    // ---- 缺 frames：同样是**注册表**层（必填缺失 ⇒ -32602，与所有工具同一条口径）----
    let (status, code) = call_tool_raw(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({ "instrumentId": instrument_id, "path": scratch.text("g.wav") }),
    );
    assert_eq!(status, 400);
    assert_eq!(code, Some(-32602), "缺必填参数 ⇒ -32602 (invalid params)");

    // 上一步的每一次拒绝都**不许**留下文件。
    for name in [
        "a.wav", "b.wav", "c.wav", "d.wav", "e.wav", "f.wav", "g.wav",
    ] {
        assert!(
            !scratch.join(name).exists(),
            "被拒绝的调用不得留下 `{name}`"
        );
    }
}

// ---------------------------------------------------------------------------
// 判据 d：dryRun 一位都不改，且与真做的实测数字一致
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing_and_matches_the_real_call() {
    let scratch = Scratch::new("dry");
    let sfz = scratch.write_sfz("two-regions.sfz", TWO_REGIONS);
    let mut dispatcher = dispatcher_with_project(filled_project());

    // 装载的 dryRun：不登记 ⇒ 随后的真装载拿到的还是 1 号。
    let preview = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string(), "dryRun": true }),
    );
    assert_eq!(
        preview["data"]["preview"]["regions"], 2,
        "dryRun 的预览载荷在 `data.preview` 下: {preview}"
    );
    assert_eq!(
        preview["data"]["preview"]["wouldRegister"]["instrumentId"],
        1
    );
    assert_eq!(
        dispatcher.domain().sfz_instrument_count(),
        0,
        "dryRun 不得登记"
    );

    let real = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(
        real["data"]["instrumentId"], 1,
        "dryRun 之后真装载仍是 1 号"
    );
    assert_eq!(dispatcher.domain().sfz_instrument_count(), 1);
    let instrument_id = 1u64;

    // 导出的 dryRun：同一批实测数字 + 不落盘。
    let target = scratch.join("dry-run.wav");
    let dry = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": target.display().to_string(),
            "dryRun": true,
        }),
    );
    assert!(!target.exists(), "dryRun 不得写文件");
    let would = &dry["data"]["preview"]["wouldWrite"];
    assert_eq!(would["frames"], 4_800);
    assert!(would["bytes"].as_u64().expect("bytes") > 44);
    let dry_sha = would["sha256"].as_str().expect("sha256").to_owned();

    let real = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": target.display().to_string(),
        }),
    );
    assert_eq!(
        real["data"]["sha256"].as_str().expect("sha256"),
        dry_sha,
        "同一请求的 dryRun 与真做必须是同一批字节"
    );
    assert_eq!(real["data"]["bytes"], would["bytes"]);
    assert!(target.exists(), "真做必须落盘");
}

// ---------------------------------------------------------------------------
// 判据 e：失败不留半成品（撞工程路径 / 撞锁文件）
// ---------------------------------------------------------------------------

#[test]
fn a_rejected_render_leaves_no_file_behind() {
    let scratch = Scratch::new("atomic");
    let sfz = scratch.write_sfz("two-regions.sfz", TWO_REGIONS);
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
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    let instrument_id = loaded["data"]["instrumentId"].as_u64().expect("id");

    // 输出路径 = 工程文件本身 ⇒ 必须被拒（"会毁掉工程"）。
    let clobber = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 480,
            "path": project_path.display().to_string(),
        }),
    );
    assert_eq!(clobber["status"], "error", "{clobber}");
    assert_eq!(clobber["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert!(
        !project_path.exists(),
        "工程文件路径不得被写成 WAV（而且它本来就不存在）"
    );

    // 输出路径 = 该工程的锁文件 ⇒ 同样被拒。
    // ⚠ 锁文件是 `<工程文件名>.lock`（`store::lock_path` 的 `LOCK_SUFFIX`），
    // 不是常量 `.yeban.lock` —— 我第一版猜错，判据当场把我抓出来。
    let lock_path = scratch.join("song.yeban.lock");
    let lock = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": instrument_id,
            "frames": 480,
            "path": lock_path.display().to_string(),
        }),
    );
    assert_eq!(lock["status"], "error", "{lock}");
    assert!(!lock_path.exists(), "锁文件路径不得被写成 WAV");
}

// ---------------------------------------------------------------------------
// 判据 f：会话态语义（打开另一个工程清空 registry；内容寻址复用 id）
// ---------------------------------------------------------------------------

#[test]
fn the_registry_is_session_scoped_and_content_addressed() {
    let scratch = Scratch::new("session");
    let sfz = scratch.write_sfz("two-regions.sfz", TWO_REGIONS);
    let copy = scratch.write_sfz("copy.sfz", TWO_REGIONS);
    let mut dispatcher = dispatcher_with_project(filled_project());

    let first = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": sfz.display().to_string() }),
    );
    let first_id = first["data"]["instrumentId"].as_u64().expect("id");
    assert_eq!(first["data"]["reused"], false);

    // 同一份**内容**（另一个路径）⇒ 复用同一个 id（内容寻址）。
    let again = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        serde_json::json!({ "path": copy.display().to_string() }),
    );
    assert_eq!(again["data"]["instrumentId"], first_id, "{again}");
    assert_eq!(again["data"]["reused"], true);
    assert_eq!(
        dispatcher.domain().sfz_instrument_count(),
        1,
        "同一份内容不得登记两份"
    );

    // 打开另一个工程 ⇒ 会话 registry 清空 ⇒ 旧 instrumentId 不再可用。
    dispatcher
        .domain_mut()
        .open_in_memory(scratch.join("other.yeban"), filled_project(), false)
        .expect("打开另一个工程");
    assert_eq!(dispatcher.domain().sfz_instrument_count(), 0);
    let stale = call_tool(
        &mut dispatcher,
        "yeban_render_sfz_wav",
        serde_json::json!({
            "instrumentId": first_id,
            "frames": 128,
            "path": scratch.text("stale.wav"),
        }),
    );
    assert_eq!(stale["status"], "error", "{stale}");
    assert_eq!(stale["error"]["data"]["reason"], "sfzInstrumentNotFound");
}

// ---------------------------------------------------------------------------
// 判据 g：可发现性 + 错误码词表
// ---------------------------------------------------------------------------

#[test]
fn the_two_new_tools_are_discoverable_and_use_d25_codes() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let auth = bearer(&dispatcher);
    let line = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), line);
    let listed = outcome
        .response
        .expect("响应")
        .result_value()
        .expect("result")["tools"]
        .as_array()
        .expect("tools 数组")
        .clone();
    assert_eq!(listed.len(), tools::TOOL_COUNT, "tools/list 必须列全部工具");
    for name in ["yeban_load_sfz", "yeban_render_sfz_wav"] {
        let descriptor = listed
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("tools/list 里找不到 `{name}`"));
        let spec = tools::tool(name).expect("注册表");
        assert_eq!(descriptor["annotations"]["specId"], spec.spec_id);
        // 每个工具都必须声明公共的两个参数（契约要求）。
        let properties = &descriptor["inputSchema"]["properties"];
        assert!(properties["dryRun"].is_object(), "{name} 缺 dryRun");
        assert!(
            properties["idempotencyKey"].is_object(),
            "{name} 缺 idempotencyKey"
        );
        // 错误码必须全部落在 `ADR-0001` D25 的 20 值联集里（⛔ 不发明新码）。
        for code in spec.errors {
            assert!(
                code.is_schema_contract(),
                "{name} 声明了联集之外的错误码 {code}"
            );
        }
    }
    assert_eq!(
        tools::TOOL_COUNT,
        tools::DOCUMENTED_TOOL_COUNT + tools::EXTENSION_TOOL_COUNT
    );
    assert_eq!(tools::DOCUMENTED_TOOL_COUNT, 10, "规范十工具不得改动");
    assert!(
        tools::EXTENSION_NAMES.contains(&"yeban_load_sfz"),
        "扩展清单必须包含新工具"
    );
    assert_eq!(tools::EXTENSION_NAMES.len(), tools::EXTENSION_TOOL_COUNT);
}
