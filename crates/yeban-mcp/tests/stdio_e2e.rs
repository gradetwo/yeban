//! [ROAD-M4-002] **端到端**判据: 真的 `spawn` `yeban-mcp` 二进制, 走 stdio 说 JSON-RPC。
//!
//! 为什么需要它: 台账 `ROAD-M4-002` 记的缺口是 `grep -rn 'CARGO_BIN_EXE_yeban-mcp' crates/` **0 命中** ——
//! 在这条判据之前, 没有任何测试 spawn 过这个二进制; 被验证的只是 `serve_lines` 的单元行为,
//! **不是**「一个真实的 MCP 客户端通过 stdio 与它对话」。
//!
//! 四层, 每层都有一条会红的断言:
//! 1. `tools_list_over_stdio_is_real` —— 真进程、真 stdout 上取到工具数组(≥12), 且点名工具在场;
//! 2. `open_then_query_over_stdio_succeeds` —— 写一份**真容器**工程, 经 stdio 打开并查询, 断言 `status=success` + `data`;
//! 3. `unknown_path_is_refused_has_teeth` —— 路径不存在时必须得到 `FILE_NOT_FOUND`(判据有牙: 否则 2 可能是假绿);
//! 4. `initialize_is_not_implemented_yet` —— **故意**的负向断言, 记录仍然存在的缺口: 二进制 dispatch 不实现 MCP 握手
//!    `initialize`(返回 `-32601`)。谁实现了握手, 这条会红 ⇒ 那时请把台账 `ROAD-M4-002` 同步改掉, **不要删掉本测试**。
//!
//! 机械边界: 不连声卡、不建引擎、不读写用户 `~/.yeban`(令牌与工程都落在仓库内 `target/` 临时目录)。
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;
use yeban_model::CommitGraph;
use yeban_model::container::write_project_container;

/// 仓库内临时目录(沙箱只允许写工作区; 也避免污染用户 `~/.yeban`)。
fn scratch() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/e2e-scratch");
    std::fs::create_dir_all(&dir).expect("建 scratch 目录");
    dir
}

/// 起一次 stdio 会话: 写请求 → 关 stdin(EOF) → 读全部响应行。
/// **先关 stdin 再读**, 因此不存在「边写边读」的管道死锁。
fn session(requests: &[String]) -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_yeban-mcp");
    let token = scratch().join(format!("token-{}.txt", std::process::id()));
    let mut child = Command::new(bin)
        .args(["--stdio", "--token-file"])
        .arg(&token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn yeban-mcp");
    {
        let mut stdin = child.stdin.take().expect("子进程 stdin");
        for r in requests {
            writeln!(stdin, "{r}").expect("写请求行");
        }
    } // stdin 掉出作用域 ⇒ 关闭 ⇒ 子进程读到 EOF 后自行退出
    let out = child.wait_with_output().expect("等待子进程");
    assert!(
        out.status.success(),
        "yeban-mcp 退出码非 0: {:?}",
        out.status
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("响应行不是 JSON ({e}): {l}")))
        .collect()
}

fn req(id: u32, method: &str, params: Value) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// 写一份**真容器**工程(与既有夹具同一条路径: `write_project_container`), 返回路径。
fn write_container_project(name: &str) -> PathBuf {
    let project = yeban_model::samples::filled_project();
    let history = serde_json::to_vec(&CommitGraph::new()).expect("空图谱 JSON");
    let bytes = write_project_container(&project, &history, &BTreeMap::new()).expect("写真容器");
    let path = scratch().join(name);
    std::fs::write(&path, bytes).expect("写容器工程");
    path
}

#[test]
fn tools_list_over_stdio_is_real() {
    let res = session(&[req(2, "tools/list", serde_json::json!({}))]);
    let r = res.first().expect("至少一行响应");
    assert_eq!(r["id"], 2, "响应必须回显请求 id: {r}");
    let tools = r["result"]["tools"].as_array().expect("tools 必须是数组");
    assert!(tools.len() >= 12, "契约工具应 ≥12 个, 实际 {}", tools.len());
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for want in ["yeban_open_project", "yeban_query_project", "yeban_undo"] {
        assert!(
            names.contains(&want),
            "工具 {want} 不在 tools/list 里: {names:?}"
        );
    }
}

#[test]
fn open_then_query_over_stdio_succeeds() {
    let path = write_container_project("e2e-open-then-query.yeban");
    let open = req(
        2,
        "tools/call",
        serde_json::json!({"name": "yeban_open_project", "arguments": {"path": path.display().to_string()}}),
    );
    let query = req(
        3,
        "tools/call",
        serde_json::json!({"name": "yeban_query_project", "arguments": {}}),
    );
    let res = session(&[open, query]);
    let o = res.iter().find(|v| v["id"] == 2).expect("应有 id=2 的响应");
    assert!(o.get("error").is_none(), "打开不应有 JSON-RPC 层错误: {o}");
    assert_eq!(
        o["result"]["status"], "success",
        "打开真容器工程应成功: {o}"
    );
    let q = res.iter().find(|v| v["id"] == 3).expect("应有 id=3 的响应");
    assert_eq!(q["result"]["status"], "success", "查询应成功: {q}");
    assert!(q["result"].get("data").is_some(), "查询应带回 data: {q}");
}

#[test]
fn unknown_path_is_refused_has_teeth() {
    let missing = scratch().join("definitely-not-here.yeban");
    let _ = std::fs::remove_file(&missing);
    let open = req(
        4,
        "tools/call",
        serde_json::json!({"name": "yeban_open_project", "arguments": {"path": missing.display().to_string()}}),
    );
    let res = session(&[open]);
    let r = res.first().expect("至少一行响应");
    assert_eq!(
        r["result"]["error"]["code"], "FILE_NOT_FOUND",
        "不存在的路径必须被点名拒绝(否则上一条判据没牙): {r}"
    );
}

#[test]
fn initialize_handshake_over_stdio_succeeds() {
    // [ROAD-M4-002] 握手判据：曾经这里是**负向**断言（记录 initialize 返回 -32601 的缺口）；
    // 缺口补上后翻成正向断言 —— 谁把握手改坏，这条就会红。
    let res = session(&[req(1, "initialize", serde_json::json!({}))]);
    let r = res.first().expect("至少一行响应");
    assert!(r.get("error").is_none(), "握手不应报错: {r}");
    assert_eq!(
        r["result"]["serverInfo"]["name"], "yeban-mcp",
        "服务器自述: {r}"
    );
    assert!(
        r["result"]["protocolVersion"].as_str().is_some(),
        "必须给出协议版本: {r}"
    );
    assert!(
        r["result"]["capabilities"]["tools"].is_object(),
        "必须声明 tools 能力: {r}"
    );
}

#[test]
fn notification_produces_no_response_but_the_session_survives() {
    // [ROAD-M4-002] 规范要求 notification（没有 id）**不产生任何响应**。
    // 这条判据的来历: 我在补 initialize 时**臆断**「本服务端没做通知特判、会对 notifications/initialized 报错」,
    // 并把该臆断写进了提交信息。实测（3 行输入 ⇒ **2 行**响应、且 tools/list 仍被回答）证明它**已经**被正确处理
    // (`dispatch.rs` 的 `request.is_notification()`)。判据留在这里, 免得下次再凭印象说话。
    let res = session(&[
        req(1, "initialize", serde_json::json!({})),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        req(2, "tools/list", serde_json::json!({})),
    ]);
    assert_eq!(
        res.len(),
        2,
        "3 行输入(含 1 条 notification) 应只产生 2 行响应: {res:?}"
    );
    let ids: Vec<_> = res.iter().map(|v| v["id"].clone()).collect();
    assert!(ids.contains(&serde_json::json!(1)), "握手响应在: {ids:?}");
    assert!(
        ids.contains(&serde_json::json!(2)),
        "通知之后的 tools/list 仍被回答: {ids:?}"
    );
    assert!(
        !res.iter().any(|v| v["id"].is_null()),
        "不得为 notification 编造响应: {res:?}"
    );
}
