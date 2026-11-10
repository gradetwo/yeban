//! `--export-song-overview` 的判据：**批处理出口与工具出口是同一份数据**。
//!
//! ## 这条判据要证的句子
//!
//! > 同一份工程，`yeban-app --open X.yeban --export-song-overview X.json` 写出的 JSON
//! > 与 `tools/call get_song_overview` 响应里的 `data` 是**同一条数据** ——
//! > 去掉两处**刻意**的差异（协议装饰 `commitModel`、本出口独有的 `renderMode`）
//! > 之后，规范序 JSON **逐字节相同**。
//!
//! ## 为什么这条必须存在
//!
//! 这个开关存在的全部理由是"没有显示器也能算总览"；如果它算出来的数与 GUI / 工具那几个
//! 口径不同（哪怕只差 0.1 LUFS 的取整），它就不是"同一条数据的第二个出口"，而是
//! **第二个真相源** —— 那比没有这个开关更糟。因此判据不是"跑通了就行"，而是
//! **两条路各跑一遍再逐字段对账**。
//!
//! ## 怎么变红
//!
//! | 改哪里 | 哪一条红 |
//! | :--- | :--- |
//! | `song_overview_export` 里绕过 `rounded` 自己取整 | `the_batch_export_is_the_same_data_as_the_tool_response` |
//! | 把 provider 换成 stub / 少传 `assets` | 同上（`provider` 字段与 `bar_series` 都对不上） |
//! | `--overview-score-only` 悄悄变成"渲染失败的回退" | `score_only_says_so_in_the_file_and_the_receipt` |
//! | 缺样本改成静音回退（不失败） | `a_missing_sampler_fails_loudly_and_writes_nothing` |
#![cfg(feature = "in-process-mcp")]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use yeban_app::cli::{self, CliError};
use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
use yeban_app::song_overview_export::{OverviewExportError, RenderMode, export_song_overview};
use yeban_mcp::transport::http::MCP_PATH;
use yeban_model::SamplerBinding;
use yeban_model::samples::demo_project;

const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 每个判据一份独立临时目录（失败时不互相污染，也不会留在仓库里）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-overview-export-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 真环回 socket 上的一次 `tools/call`，返回响应体。
fn call(address: SocketAddr, bearer: &str, body: &str) -> Value {
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面");
    stream.set_read_timeout(Some(IO_TIMEOUT)).expect("读超时");
    stream.set_write_timeout(Some(IO_TIMEOUT)).expect("写超时");
    let head = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\nAuthorization: {bearer}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("读响应");
    let body = match raw.find("\r\n\r\n") {
        Some(index) => &raw[index + 4..],
        None => panic!("响应没有头体分隔: {raw}"),
    };
    serde_json::from_str(body).unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {body}"))
}

/// 规范序 JSON（键排序、无空白）—— 判据只比**数据**，不比缩进。
fn canonical(value: &Value) -> String {
    serde_json::to_string(&sorted(value)).expect("序列化")
}

/// 递归按键排序（`serde_json::Value` 的对象是 `Map`，`to_string` 不保证有序）。
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// 跑一次批处理导出，返回 stdout 的每一行。
fn run_export(args: &[&str]) -> Result<Vec<String>, CliError> {
    let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let options = cli::parse(&argv).map_err(|error| CliError::Usage(error.to_string()))?;
    cli::run_batch(&options)
}

/// **核心判据**：批处理出口与工具出口是同一份数据。
///
/// 论证顺序：
/// 1. 挂一个**内存会话**控制面（工程 = `demo_project()`，与 `--project-sample default` 同一份）；
/// 2. 经真 socket 调 `get_song_overview` ⇒ 拿到工具的 `data`；
/// 3. 跑批处理 `--project-sample default --export-song-overview <tmp>.json`；
/// 4. 两边去掉 `commitModel` / `renderMode` 之后，规范序 JSON 必须**逐字节相同**；
/// 5. 回执行的读数必须与文件内容**对得上**（小节数、provider、render 口径）。
#[test]
fn the_batch_export_is_the_same_data_as_the_tool_response() {
    let dir = scratch("parity");
    let target = dir.join("overview.json");

    // ---- 1/2. 工具出口（形态 A：真 socket） ----
    let mount = InProcessMcp::start_for_project(
        true,
        demo_project(),
        SessionSource::InMemory(PathBuf::from("<test:overview-parity>")),
        yeban_model::samples::demo_project_assets(),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let bearer = format!("Bearer {}", mount.token().expose());
    let reply = call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_song_overview","arguments":{"resolution":"bar"}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    let mut tool_data = reply["result"]["data"].clone();
    mount.stop().expect("停机必须成功");

    // ---- 3. 批处理出口 ----
    let lines = run_export(&[
        "--project-sample",
        "default",
        "--export-song-overview",
        &target.display().to_string(),
    ])
    .expect("演示工程的总览必须能算出来");
    let text = std::fs::read_to_string(&target).expect("JSON 必须真的落盘");
    let mut cli_data: Value = serde_json::from_str(&text).expect("落盘的必须是 JSON");

    // ---- 4. 去掉两处刻意的差异，逐字节对账 ----
    let commit_model = tool_data
        .as_object_mut()
        .expect("data 是对象")
        .remove("commitModel");
    assert_eq!(
        commit_model,
        Some(Value::from("none")),
        "前提：只读工具的协议装饰恒为 `none`（ADR-0032 D1）—— 判据据此确认它只是装饰"
    );
    let render_mode = cli_data
        .as_object_mut()
        .expect("data 是对象")
        .remove("renderMode");
    assert_eq!(
        render_mode,
        Some(Value::from("full")),
        "缺省必须是全量渲染（⛔ 不是悄悄降级成纯谱面）"
    );

    assert_eq!(
        canonical(&cli_data),
        canonical(&tool_data),
        "⭐ 批处理出口与 `get_song_overview` 的 `data` 必须是同一份数据\
         （差一个字段/一个取整就是第二个真相源）"
    );

    // ---- 5. 回执的读数与文件对得上（不是把入参抄一遍） ----
    let receipt = lines
        .iter()
        .find(|line| line.starts_with("exported-song-overview:"))
        .unwrap_or_else(|| panic!("缺少回执行:\n{}", lines.join("\n")));
    let bars = cli_data["bar_series"]
        .as_array()
        .expect("bar_series 是数组")
        .len();
    assert_eq!(bars, 3, "演示工程的已知答案：3 小节");
    assert!(
        receipt.contains(&format!("bars={bars}")),
        "回执的小节数必须是实测值: {receipt}"
    );
    assert!(
        receipt.contains(&format!("bytes={}", text.len())),
        "回执的字节数必须是落盘后的实测值: {receipt}"
    );
    assert!(
        receipt.contains("provider=real") && receipt.contains("render=full"),
        "回执必须说清是哪条算法、哪种渲染口径: {receipt}"
    );
}

/// **第二判据**：`--overview-score-only` 是**显式**选择，且它在文件与回执里都**自报家门**。
///
/// 这一档存在的理由：GUI 面板在渲染失败时会回退到纯谱面（它总得画点什么），而批处理
/// 产出的是一份**交付物** —— 如果"渲染失败所以全是哨兵值"的文件和正常文件长得一模一样，
/// 那就是假的。因此这一档必须只由开关触发，且必须留痕。
#[test]
fn score_only_says_so_in_the_file_and_the_receipt() {
    let dir = scratch("score-only");
    let target = dir.join("score.json");
    let lines = run_export(&[
        "--project-sample",
        "default",
        "--overview-score-only",
        "--export-song-overview",
        &target.display().to_string(),
    ])
    .expect("纯谱面也必须有产出");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&target).expect("落盘")).expect("JSON");

    assert_eq!(
        data["renderMode"], "score-only",
        "文件里必须自报口径: {data}"
    );
    assert_eq!(
        data["provider"], "real",
        "纯谱面也走真 provider 的映射（⛔ 不是另一份 stub）"
    );
    let receipt = lines
        .iter()
        .find(|line| line.starts_with("exported-song-overview:"))
        .expect("必须有回执行");
    assert!(
        receipt.contains("render=score-only"),
        "回执必须说清这一份没有渲染: {receipt}"
    );
    // 纯谱面没有音频 ⇒ 每个小节的响度都是 −70 哨兵 ⇒ 回执不报"响度范围"。
    assert!(
        receipt.contains("lufs=none"),
        "⛔ 不许把哨兵值 −70 当成一个真实测量值打出来: {receipt}"
    );
}

/// **第三判据**：采样器读不到 ⇒ **响亮失败**，且**不留半成品**。
///
/// 这一条直接打模块（不经 CLI）：批处理路径里"失败"的落点就是
/// [`OverviewExportError::Render`]，它带着 provider 的 `Fault`（稳定码 + 结构化 `data`），
/// 由 `CliError::SongOverviewExport` 带到退出码 5。
#[test]
fn a_missing_sampler_fails_loudly_and_writes_nothing() {
    let dir = scratch("missing-sampler");
    let target = dir.join("never.json");

    let mut project = demo_project();
    let track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("演示工程有非 master 音轨");
    project.tracks.get_mut(&track).expect("刚找到的轨").sampler = Some(SamplerBinding {
        path: "这份定义不存在.sfz".to_owned(),
    });

    let error = export_song_overview(&project, &target, None, &[], RenderMode::Full)
        .expect_err("采样器读不到必须失败（⛔ 不静默回退成纯谱面）");
    match error {
        OverviewExportError::Render(fault) => {
            assert_eq!(
                fault.domain_code(),
                Some(yeban_mcp::tools::ErrorCode::RenderFailed),
                "稳定码必须是 RENDER_FAILED: {fault:?}"
            );
        }
        other => panic!("必须是 provider 的失败，实得 {other:?}"),
    }
    assert!(
        !target.exists(),
        "⭐ 失败不得留下半成品（⛔ 一个空文件也不行）: {}",
        target.display()
    );

    // 反向腿：同一个工程，**显式**要纯谱面 ⇒ 成功（这正是 `--overview-score-only` 的意义）。
    let report = export_song_overview(&project, &target, None, &[], RenderMode::ScoreOnly)
        .expect("纯谱面不吃采样器");
    assert_eq!(report.mode, RenderMode::ScoreOnly);
    assert!(
        target.exists(),
        "反向腿必须真的写出来（否则上一条的 `!exists` 就不是这条失败造成的）"
    );
}

/// 判据四：**落盘路径不可写** ⇒ 失败且 `path` 指向目标（与导出族同款）。
#[test]
fn an_unwritable_target_fails_with_the_intended_path() {
    let dir = scratch("unwritable");
    // 父目录不存在 ⇒ `write_file_atomically` 的同目录临时文件建不出来。
    let target = dir.join("no-such-dir").join("x.json");
    let error = export_song_overview(&demo_project(), &target, None, &[], RenderMode::Full)
        .expect_err("父目录不存在必须失败");
    match error {
        OverviewExportError::Save { path, .. } => assert_eq!(
            path,
            Path::new(&target),
            "失败里带的必须是**目标路径**（用户要去看的就是它）"
        ),
        other => panic!("必须是落盘失败，实得 {other:?}"),
    }
    assert!(!target.exists(), "失败不得留下半成品");
}

/// **第五判据**：整曲渲染**静音**时，回执多一行 `overview-silent:`（⛔ 不改退出码）。
///
/// 这一条的来历（实测，2026-11-10）：一份真实工程（57 小节）算出来的总览**全静音**，
/// 而导出**成功**、退出码 0、JSON 里每个 `lufs_short` 都是 −70 哨兵 —— 没有一个字
/// 提示"这份数是静音的"。缺样本在 provider 里是"跳过该样本"（引擎标 `NO_SAMPLE`），
/// 不是错误 ⇒ 静音必须**自己说出来**。
///
/// 夹具用一个 `sample=*silence` 的 SFZ：它是"合法、读得到、但一个采样都不发声"的
/// 那一档 —— 正好是这条判据要的形态（⛔ 不是造一个坏工程来触发错误）。
#[test]
fn a_silent_full_render_says_so_without_failing() {
    let dir = scratch("silent");
    let sfz = dir.join("silence.sfz");
    std::fs::write(&sfz, "<region> sample=*silence\n").expect("写夹具 SFZ");

    let mut project = demo_project();
    let track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("演示工程有非 master 音轨");
    project.tracks.get_mut(&track).expect("刚找到的轨").sampler = Some(SamplerBinding {
        path: sfz.display().to_string(),
    });

    let target = dir.join("silent.json");
    let report = export_song_overview(&project, &target, Some(&dir), &[], RenderMode::Full)
        .expect("静音不是失败（空工程 / 全休止都合法）⇒ 必须成功");
    assert!(
        report.is_silent(),
        "前提：`*silence` 音色必须真的渲染出静音（lufs={:?}）",
        report.lufs
    );
    let note = report.silent_note().expect("静音必须有诊断行");
    assert!(
        note.starts_with("overview-silent:"),
        "诊断行的前缀是契约（可 grep）: {note}"
    );
    assert!(
        note.contains("57") || note.contains(&report.bars.to_string()),
        "诊断行必须带小节数（实测值）: {note}"
    );

    // 反向腿：同一份工程要纯谱面 ⇒ 不静音（它本来就没有音频），**不该**多那一行。
    let score = export_song_overview(&project, &target, Some(&dir), &[], RenderMode::ScoreOnly)
        .expect("纯谱面");
    assert!(
        !score.is_silent() && score.silent_note().is_none(),
        "`score-only` 本来就没有音频 ⇒ ⛔ 不该报「静音」"
    );
}
