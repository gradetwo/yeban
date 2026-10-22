//! ⭐ 任务单 `#219`：**内嵌装配**的 `mcp_mount::session_dispatcher` 真的驱动
//! `yeban_load_sfz` / `yeban_render_sfz_wav`（⛔ 不是"结构共享"，⛔ 不是测试里再复制一份装配）。
//!
//! ## 量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 来源 | 单位 |
//! | :--- | :--- | :--- |
//! | `regions` | `data.regions`（`yeban_load_sfz`） | 区域数 |
//! | `silentRegionCount` | 同上 | `sample=*silence` 区域数 |
//! | `sampleFilesReadCount` | 同上 | 真的读盘解出 PCM 的样本份数 |
//! | `instrumentId` / `reused` | 同上 | 会话内乐器身份 / 是否内容寻址复用 |
//! | `notesUsingBuiltinSilence` | `data.notesUsingBuiltinSilence`（`yeban_render_sfz_wav`） | 落在内建静音上的音符数 |
//! | `notesUsingFileSample` | 同上 | 落在真文件样本上的音符数（正对照） |
//! | `peak` | 同上（渲染器给出的 `max abs(x)`） | 归一化浮点（1.0 = 满刻度） |
//! | `sha256` / `bytes` | 同上（导出 WAV 的字节摘要 / 长度） | 十六进制 / 字节 |
//!
//! ## 装配（这一条是整单的重点）
//!
//! 这里的 `Dispatcher` **只**来自 [`super::session_dispatcher`] —— 生产路径
//! [`super::InProcessMcp::start_for_project`] 调用的**同一个函数**（见 `src/mcp_mount.rs`
//! 的 `start_for_project` 与 `fn session_dispatcher`）。测试**不**自己 `Dispatcher::new`：
//! 一旦复制装配，"结构共享"就又回来了，而本单的存在理由正是把它升级成"真驱动"。
//!
//! 一条子模块（同文件树的 `mcp_mount/embedded_sfz_tools.rs`）因此能直接调用父模块的
//! 私有（`pub(crate)`）函数：可见性不扩大，调用点是同一个符号。
//!
//! ## 两条装配的差集（`#218` 给的）
//!
//! | 维度 | stdio 装配（`yeban-mcp.rs`） | 内嵌装配（本模块） |
//! | :--- | :--- | :--- |
//! | `Dispatcher` | `Dispatcher::new(token, AppAdmin, Production)` | `Dispatcher::new(token, all, Production)` |
//! | 时钟 | `domain.set_now_ms(now_ms())` | **不注入**（默认 `0`） |
//! | 会话 | 经 `yeban_open_project` 工具打开 | `open_in_memory(path, project, read_only)` |
//! | 通道 | `Channel::Stdio`（`LocalProcess`） | `Channel::Http` ＋ Bearer |
//!
//! 差别只有这三项；`read_only` 只闸落盘（`plan_save`），时钟只进 `yeban_render_master`
//! 的报文（`renderedAtUnixMs`）—— 两条都不参与 SFZ 的装载 / 渲染。本判据把这句话
//! 变成读数，而不是再抄一遍注释。
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证用户机器上的真 SFZ 库（夹具全部写在 `std::env::temp_dir()`
//! 下，`Drop` 删除）。⛔ 不证 socket 层（那在 `tests/in_process_mcp.rs`）。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{AssetHash, ClipContent, EntityId, YebanProjectV1};

use super::session_dispatcher;

/// 母带 / SFZ 都不写 `bext`，1 LSB 是 24-bit 抖动底（解码腿按 `Int24 / 8_388_607` 归一）。
const ONE_LSB_I24: f64 = 1.0 / 8_388_607.0;

/// stdio 装配注入的时钟（`yeban-mcp` 的 `now_ms()` 是系统时间；判据用固定值）。
const INJECTED_NOW_MS: u64 = 1_760_000_000_000;

/// 夹具 SFZ：一条内建静音 ＋ 一条真样本（与 `#218` / app `#217` 同一形状）。
const MIXED_SFZ: &str = "<region> key=60 sample=*silence\n<region> key=72 sample=tone.wav\n";

/// 临时夹具目录（`Drop` 删掉自己；工作树里不留临时文件）。
pub(super) struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub(super) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-app-embed-sfz-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    pub(super) fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 夹具目录本身（CLI 腿把它当 `project_dir` 传进去）。
    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(super) fn write_sfz(&self, name: &str, body: &str) -> PathBuf {
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
pub(super) fn write_constant_wav(path: &Path, value: f64, frames: usize, sample_rate: u32) {
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
/// ⚠ 与 `#218` 同一条理由：`filled_project()` 的第 4 个音符（`pitch=72`）带
/// `probability=Some(0.75)` ⇒ 固定 `rng_seed` 下会挑一个随机结果；清掉之后
/// `pitch=72` 必然触发，正对照才不会红在"没有音符被排程"。
pub(super) fn project_without_note_probability() -> YebanProjectV1 {
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

fn fixture_sfz(scratch: &Scratch) -> PathBuf {
    write_constant_wav(&scratch.join("tone.wav"), 0.25, 480, 48_000);
    scratch.write_sfz("mixed.sfz", MIXED_SFZ)
}

/// ⭐ **内嵌装配本体**：只调 [`session_dispatcher`]（生产路径的同一个符号）。
pub(super) fn embedded(scratch: &Scratch, read_only: bool) -> Dispatcher {
    session_dispatcher(
        &project_without_note_probability(),
        &scratch.join("session.yeban"),
        read_only,
        Vec::new(),
    )
    .expect("内嵌会话装配")
}

/// **stdio 装配的镜像**（`crates/yeban-mcp/src/bin/yeban-mcp.rs` 的三行 ＋ 会话注入）。
///
/// 只出现在**对照**里：判据 5 要证明两套装配给出同一批读数。⚠ 它不是被测对象 ——
/// 被测对象永远是 [`embedded`]。
pub(super) fn stdio_shaped(scratch: &Scratch, now_ms: u64, read_only: bool) -> Dispatcher {
    let mut dispatcher = Dispatcher::new(
        BearerToken::generate().token,
        ScopeSet::from_scopes([Scope::AppAdmin]),
        RunMode::Production,
    );
    dispatcher.domain_mut().set_now_ms(now_ms);
    dispatcher
        .domain_mut()
        .open_in_memory(
            scratch.join("session.yeban"),
            project_without_note_probability(),
            read_only,
        )
        .expect("注入规范工程");
    dispatcher
}

pub(super) fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 走**各自通道**的一条 `tools/call`：HTTP 带 Bearer，stdio 不带头（`LocalProcess`）。
pub(super) fn call_tool(
    dispatcher: &mut Dispatcher,
    channel: Channel,
    name: &str,
    arguments: Value,
) -> Value {
    let authorization = (channel == Channel::Http).then(|| bearer(dispatcher));
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(channel, authorization.as_deref(), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 判据 ①②：**内嵌装配**真的把两个工具都驱动到（读数 = 区域 / 静音 / 峰值 ＋ 正对照）。
#[test]
fn the_embedded_session_dispatcher_drives_both_sfz_tools() {
    let scratch = Scratch::new("drive");
    let sfz = fixture_sfz(&scratch);
    let mut dispatcher = embedded(&scratch, false);

    // 判据 ①：这个 Dispatcher 的形状就是 `session_dispatcher` 造出来的那一份。
    assert_eq!(dispatcher.mode(), RunMode::Production);
    assert_eq!(
        dispatcher.domain().now_ms(),
        0,
        "#218 的事实：内嵌装配从不调用 set_now_ms"
    );

    let loaded = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_load_sfz",
        json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    let load = &loaded["data"];
    println!(
        "[app/embed/load] regions={} silent={} samples={} instrumentId={} reused={} source={:?}",
        load["regions"],
        load["silentRegionCount"],
        load["sampleFilesReadCount"],
        load["instrumentId"],
        load["reused"],
        load["sampleSource"],
    );
    assert_eq!(load["regions"], 2, "两条区域: {load}");
    assert_eq!(load["silentRegionCount"], 1, "`*silence` 区域数: {load}");
    assert_eq!(
        load["sampleFilesReadCount"], 1,
        "只有 tone.wav 这一份真样本: {load}"
    );
    let instrument_id = load["instrumentId"].as_u64().expect("instrumentId");
    assert_eq!(
        dispatcher.granted().len(),
        ScopeSet::all().len(),
        "内嵌装配给的是全量作用域（与 stdio 的 AppAdmin 不同，但都覆盖 app:admin）"
    );

    // 判据 ②：`note: 60` ⇒ 只落在内建静音区域（峰值是抖动底）。
    let silence_path = scratch.join("silence-only.wav");
    let silent = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_render_sfz_wav",
        json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": silence_path.display().to_string(),
            "note": 60,
        }),
    );
    assert_eq!(silent["status"], "success", "{silent}");
    let silence = &silent["data"];
    let silence_peak = silence["peak"].as_f64().expect("peak");
    println!(
        "[app/embed/render/silence] scheduled={} builtin_silence={} file={} sine={} peak={silence_peak:e} bytes={} sha256={:?}",
        silence["notesScheduled"],
        silence["notesUsingBuiltinSilence"],
        silence["notesUsingFileSample"],
        silence["notesUsingSineFallback"],
        silence["bytes"],
        silence["sha256"],
    );
    assert_eq!(silence["notesScheduled"], 1, "{silence}");
    assert_eq!(
        silence["notesUsingBuiltinSilence"], 1,
        "落在内建静音上的音符数: {silence}"
    );
    assert_eq!(silence["notesUsingFileSample"], 0, "⛔ 这条没有文件样本");
    assert_eq!(silence["notesUsingSineFallback"], 0, "⛔ 不许回退内存正弦");
    assert!(
        silence_peak <= ONE_LSB_I24,
        "内建静音只应有抖动底 (≤ 1 LSB), 实得 {silence_peak:e}"
    );
    let silence_bytes = std::fs::read(&silence_path).expect("读回静音 WAV");
    assert_eq!(silence["bytes"], silence_bytes.len(), "报文与落盘字节数");
    assert_eq!(
        silence["sha256"],
        AssetHash::of_bytes(&silence_bytes).as_str(),
        "报文摘要必须来自真的落盘字节"
    );

    // 正对照：`note: 72` ⇒ 真样本区域必须出声（否则上面的 0 可能是"夹具全不匹配"）。
    let tone_path = scratch.join("tone-only.wav");
    let tone = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_render_sfz_wav",
        json!({
            "instrumentId": instrument_id,
            "frames": 96_000,
            "path": tone_path.display().to_string(),
            "note": 72,
        }),
    );
    assert_eq!(tone["status"], "success", "{tone}");
    let tone = &tone["data"];
    let tone_peak = tone["peak"].as_f64().expect("peak");
    println!(
        "[app/embed/render/tone] scheduled={} builtin_silence={} file={} sine={} peak={tone_peak:.6}",
        tone["notesScheduled"],
        tone["notesUsingBuiltinSilence"],
        tone["notesUsingFileSample"],
        tone["notesUsingSineFallback"],
    );
    assert_eq!(tone["notesUsingBuiltinSilence"], 0, "{tone}");
    assert_eq!(tone["notesUsingFileSample"], 1, "真样本必须被用上: {tone}");
    assert_eq!(tone["notesUsingSineFallback"], 0, "{tone}");
    assert!(
        tone_peak > 0.01,
        "正对照：真样本区域必须出声（峰值实得 {tone_peak:e}）"
    );
}

/// 判据 ③：`read_only` **只闸落盘**，⛔ 不闸这两个工具。
///
/// 有牙的一侧：同一条内嵌装配上 `yeban_save_project` 在 `read_only = true` 时必须被
/// `IO_ERROR` 拒绝（⇒ 这条判据能证明 `read_only` 真被传进去了，而不是恒 `false`）。
#[test]
fn read_only_gates_saving_but_not_the_two_sfz_tools() {
    let scratch = Scratch::new("read-only");
    let sfz = fixture_sfz(&scratch);
    let wav = scratch.join("read-only.wav");
    let mut readings: Vec<(Value, Value)> = Vec::new();

    for read_only in [true, false] {
        let mut dispatcher = embedded(&scratch, read_only);
        let loaded = call_tool(
            &mut dispatcher,
            Channel::Http,
            "yeban_load_sfz",
            json!({ "path": sfz.display().to_string() }),
        );
        assert_eq!(
            loaded["status"], "success",
            "read_only={read_only}: {loaded}"
        );
        let instrument_id = loaded["data"]["instrumentId"]
            .as_u64()
            .expect("instrumentId");
        let rendered = call_tool(
            &mut dispatcher,
            Channel::Http,
            "yeban_render_sfz_wav",
            json!({
                "instrumentId": instrument_id,
                "frames": 4_800,
                "path": wav.display().to_string(),
                "note": 60,
            }),
        );
        assert_eq!(
            rendered["status"], "success",
            "read_only={read_only}: {rendered}"
        );
        println!(
            "[app/embed/read-only] read_only={read_only} regions={} silent={} builtin_silence={} \
             notesUsingFileSample={} peak={:?} sha256={:?}",
            loaded["data"]["regions"],
            loaded["data"]["silentRegionCount"],
            rendered["data"]["notesUsingBuiltinSilence"],
            rendered["data"]["notesUsingFileSample"],
            rendered["data"]["peak"],
            rendered["data"]["sha256"],
        );
        readings.push((loaded["data"].clone(), rendered["data"].clone()));

        // 反向对照：`read_only` 不是被吞掉的参数 —— 落盘必须真的被拒 / 放行。
        let saved = call_tool(
            &mut dispatcher,
            Channel::Http,
            "yeban_save_project",
            json!({ "force": true }),
        );
        if read_only {
            assert_eq!(saved["status"], "error", "只读会话必须拒绝落盘: {saved}");
            assert_eq!(saved["error"]["code"], "IO_ERROR", "{saved}");
            assert!(
                !scratch.join("session.yeban").exists(),
                "只读会话一个字节都不许写"
            );
            println!(
                "[app/embed/read-only] read_only=true save={}",
                saved["error"]["code"]
            );
        } else {
            assert_eq!(saved["status"], "success", "写会话必须能落盘: {saved}");
            assert!(
                scratch.join("session.yeban").is_file(),
                "写会话必须真的写出工程文件"
            );
            println!("[app/embed/read-only] read_only=false saved=true");
        }
    }

    assert_eq!(
        readings[0].0, readings[1].0,
        "`yeban_load_sfz` 的读数不得随 read_only 变"
    );
    assert_eq!(
        readings[0].1, readings[1].1,
        "`yeban_render_sfz_wav` 的读数不得随 read_only 变"
    );
}

/// 判据 ④⑤：内嵌装配（**不**注入时钟）与 stdio 装配（注入时钟）**逐项相同**。
#[test]
fn the_embedded_dispatcher_matches_the_stdio_assembly_field_by_field() {
    let scratch = Scratch::new("compare");
    let sfz = fixture_sfz(&scratch);
    // ⚠ 两条装配用**同一个**输出路径：`path` 也是报文的一部分，换路径会让
    // "只差装配"的对照变成"差在路径上"。
    let wav = scratch.join("compare.wav");

    let mut embedded = embedded(&scratch, false);
    let mut stdio = stdio_shaped(&scratch, INJECTED_NOW_MS, false);

    // 时钟确实不同：内嵌 = 0（从未注入），stdio = 注入值。
    assert_eq!(embedded.domain().now_ms(), 0);
    assert_eq!(stdio.domain().now_ms(), INJECTED_NOW_MS);

    let embedded_load = call_tool(
        &mut embedded,
        Channel::Http,
        "yeban_load_sfz",
        json!({ "path": sfz.display().to_string() }),
    );
    let stdio_load = call_tool(
        &mut stdio,
        Channel::Stdio,
        "yeban_load_sfz",
        json!({ "path": sfz.display().to_string() }),
    );
    assert_eq!(embedded_load["status"], "success", "{embedded_load}");
    assert_eq!(stdio_load["status"], "success", "{stdio_load}");
    assert_eq!(
        embedded_load["data"]["instrumentId"], stdio_load["data"]["instrumentId"],
        "同一份内容 ⇒ 同一个会话内 id"
    );

    let instrument_id = embedded_load["data"]["instrumentId"]
        .as_u64()
        .expect("instrumentId");
    let embedded_render = call_tool(
        &mut embedded,
        Channel::Http,
        "yeban_render_sfz_wav",
        json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": wav.display().to_string(),
            "note": 60,
        }),
    );
    let stdio_render = call_tool(
        &mut stdio,
        Channel::Stdio,
        "yeban_render_sfz_wav",
        json!({
            "instrumentId": instrument_id,
            "frames": 4_800,
            "path": wav.display().to_string(),
            "note": 60,
        }),
    );
    assert_eq!(embedded_render["status"], "success", "{embedded_render}");
    assert_eq!(stdio_render["status"], "success", "{stdio_render}");

    for key in [
        "regions",
        "silentRegionCount",
        "sampleFilesReadCount",
        "instrumentId",
        "reused",
        "sampleSource",
    ] {
        println!(
            "[app/embed/compare/load] {key}: embedded={} stdio={}",
            embedded_load["data"][key], stdio_load["data"][key]
        );
    }
    for key in [
        "notesScheduled",
        "notesUsingBuiltinSilence",
        "notesUsingFileSample",
        "notesUsingSineFallback",
        "frames",
        "bytes",
        "sha256",
        "peak",
        "sampleSource",
    ] {
        println!(
            "[app/embed/compare/render] {key}: embedded={} stdio={}",
            embedded_render["data"][key], stdio_render["data"][key]
        );
    }

    assert_eq!(
        embedded_load["data"], stdio_load["data"],
        "两条装配的 `yeban_load_sfz` 报文必须逐项相同"
    );
    assert_eq!(
        embedded_render["data"], stdio_render["data"],
        "两条装配的 `yeban_render_sfz_wav` 报文必须逐项相同"
    );
    // 报文相同 ⇒ 墙钟（只有 stdio 注入）在这两个工具的报文里**没有**任何字段。
    assert!(
        embedded_render["data"].get("renderedAtUnixMs").is_none(),
        "SFZ 导出不写墙钟: {}",
        embedded_render["data"]
    );
}
