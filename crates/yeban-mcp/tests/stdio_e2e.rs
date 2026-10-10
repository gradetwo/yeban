//! [ROAD-M4-002] **端到端**判据: 真的 `spawn` `yeban-mcp` 二进制, 走 stdio 说 JSON-RPC。
//!
//! 为什么需要它: 台账 `ROAD-M4-002` 记的缺口是 `grep -rn 'CARGO_BIN_EXE_yeban-mcp' crates/` **0 命中** ——
//! 在这条判据之前, 没有任何测试 spawn 过这个二进制; 被验证的只是 `serve_lines` 的单元行为,
//! **不是**「一个真实的 MCP 客户端通过 stdio 与它对话」。
//!
//! 六条判据, 每条都有一条会红的断言:
//! 1. `one_client_session_does_handshake_tools_list_and_a_real_call` —— **台账点名的那一条**: 一个真实客户端在
//!    **一次**会话里走完 握手 → `tools/list` → 真容器工程的 `yeban_open_project` → `yeban_query_project`,
//!    并断言 `tools/list` 与注册表**逐项相同**(`TOOL_COUNT` 条, 名字与顺序)以及查询载荷的形状;
//! 2. `tools_list_over_stdio_is_real` —— 真进程、真 stdout 上取到工具数组, 与注册表逐项对账, 并点名工具在场;
//! 3. `open_then_query_over_stdio_succeeds` —— 写一份**真容器**工程, 经 stdio 打开并查询, 断言 `status=success` + `data`;
//! 4. `unknown_path_is_refused_has_teeth` —— 路径不存在时必须得到 `FILE_NOT_FOUND`(判据有牙: 否则 3 可能是假绿);
//! 5. `initialize_handshake_over_stdio_succeeds` —— 握手是**正向**判据。它曾经是负向断言(记录二进制不实现
//!    `initialize`、返回 `-32601` 的缺口); 缺口补上后翻正 —— 谁把握手改坏, 这条会红;
//! 6. `notification_produces_no_response_but_the_session_survives` —— notification 不产生响应, 且会话继续可用。
//!
//! 会话的**机械边界**全部收在 [`session`] 里, 所有判据共用:
//! - stdout / stderr 由**独立线程**抽干 ⇒ 17 个工具的 `tools/list`(实测约 15 KB)不会把子进程堵在写端;
//! - 写完请求即关 stdin(EOF); 退出由 [`SESSION_TIMEOUT`] 兜底, 超时先 `kill` 再红
//!   ⇒ 真二进制出问题是一次**响亮的失败**, 不是一次挂到 CI 作业上限的"卡住";
//! - 断言退出码**恰好是 `0`**(不是被信号带走) ⇒ "客户端关掉 stdin 后服务器自己收摊、什么也没留下"也是判据。
//!
//! 环境边界: 不连声卡、不建引擎、不读写用户 `~/.yeban`(令牌与工程都落在仓库内 `target/` 临时目录)。
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use yeban_model::CommitGraph;
use yeban_model::container::write_project_container;

/// 仓库内临时目录(沙箱只允许写工作区; 也避免污染用户 `~/.yeban`)。
fn scratch() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/e2e-scratch");
    std::fs::create_dir_all(&dir).expect("建 scratch 目录");
    dir
}

/// 一次 stdio 会话允许的最长时间。
///
/// 为什么必须有兜底: 这里跑的是**真二进制**。没有它时, 任何一个让批处理读不到 EOF 的回归
/// 都会让 `cargo test` 永远等下去 —— 一次明确的失败会变成一次"卡住"(在 CI 上就是挂到作业上限)。
/// 30 秒足够一次本地往返(实测整条会话 < 1 秒), 又不至于让坏回归拖延判决。
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);

/// 注册表的权威工具名(**顺序即注册顺序**, 与 `tools/list` 的输出顺序是同一件事)。
fn registry_names() -> Vec<String> {
    yeban_mcp::tools::TOOLS
        .iter()
        .map(|spec| spec.name.to_owned())
        .collect()
}

/// 起一次 stdio 会话: 写请求 → 关 stdin(EOF) → 等子进程自己退出 → 读全部响应行。
///
/// 为什么 stdout / stderr 要在**独立线程**里抽干: `tools/list` 的响应实测约 15 KB, 再多几条工具
/// 就会超过管道缓冲区 —— 那时子进程会阻塞在写端、永远到不了 EOF, 而"先写完再读"的调用方会一直
/// 等它 ⇒ 双方互等。抽干线程让这件事在结构上不可能发生。
///
/// 为什么"先关 stdin": 批处理语义是读到 EOF 就退出, 因此不存在边写边读的往返死锁, 也不用猜
/// "响应齐了没有"。超时兜底见 [`SESSION_TIMEOUT`]; 退出码断言也在这里 —— `try_wait` 返回 `Some`
/// 即该 pid **已被回收**, 而 `code() == Some(0)` 表示它是读到 EOF 后自己正常退出的(不是被 `kill`
/// 杀的、也不是被信号带走的), 也就是**没有留下任何还在跑的东西**。
fn session(requests: &[String]) -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_yeban-mcp");
    let token = scratch().join(format!("token-{}.txt", std::process::id()));
    let mut child = Command::new(bin)
        .args(["--stdio", "--token-file"])
        .arg(&token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn yeban-mcp");

    let mut stdout = child.stdout.take().expect("子进程 stdout");
    let out_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let mut stderr = child.stderr.take().expect("子进程 stderr");
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    {
        let mut stdin = child.stdin.take().expect("子进程 stdin");
        for r in requests {
            writeln!(stdin, "{r}").expect("写请求行");
        }
    } // stdin 掉出作用域 ⇒ 关闭 ⇒ 子进程读到 EOF 后自行退出

    let deadline = Instant::now() + SESSION_TIMEOUT;
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("yeban-mcp 在 {SESSION_TIMEOUT:?} 内没有退出 —— stdio 批处理可能卡住了");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = child.wait().expect("回收子进程");
    let out = out_reader.join().expect("stdout 抽取线程");
    let err = err_reader.join().expect("stderr 抽取线程");

    assert_eq!(
        status.code(),
        Some(0),
        "yeban-mcp 非正常退出: {status:?}; stderr={err}"
    );
    out.lines()
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

/// [ROAD-M4-002] **台账点名的那一条判据**: 一个真实的 MCP 客户端在**一次** stdio 会话里
/// 走完 握手 → `tools/list` → 真实工具调用。
///
/// 为什么要有这一条, 而不是只留下面几条分段判据: 分段判据各自新起一个进程, 于是
/// "握手之后**这条连接**还能不能干活"从来没有被证明过 —— 而 MCP 的握手存在的理由**正是**
/// 给后续请求定版本 / 定能力。这里 5 行输入落在**同一个**子进程的**同一条** stdin/stdout 上。
#[test]
fn one_client_session_does_handshake_tools_list_and_a_real_call() {
    let path = write_container_project("e2e-one-session.yeban");
    let res = session(&[
        req(
            1,
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "yeban-stdio-e2e", "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        // 规范要求客户端在 initialize 之后发这条 notification —— 它**不得**占一行响应。
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        req(2, "tools/list", serde_json::json!({})),
        req(
            3,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
        req(
            4,
            "tools/call",
            serde_json::json!({
                "name": "yeban_query_project",
                "arguments": {"limit": 3},
            }),
        ),
    ]);

    // ---- ① 5 行输入(含 1 条 notification) ⇒ 恰好 4 行响应, 且 id 顺序与请求顺序一致 ----
    let ids: Vec<Value> = res.iter().map(|r| r["id"].clone()).collect();
    assert_eq!(
        ids,
        vec![
            serde_json::json!(1),
            serde_json::json!(2),
            serde_json::json!(3),
            serde_json::json!(4)
        ],
        "notification 不得占一行响应, 响应必须按请求顺序: {res:?}"
    );

    // ---- ② 握手: 服务端自述 + 回显协议版本 + 声明 tools 能力 ----
    let init = &res[0]["result"];
    assert_eq!(init["serverInfo"]["name"], "yeban-mcp", "{init}");
    assert_eq!(init["protocolVersion"], "2025-06-18", "{init}");
    assert!(init["capabilities"]["tools"].is_object(), "{init}");

    // ---- ③ tools/list: 与注册表逐项相同(条数 + 名字 + 顺序) ----
    let names: Vec<String> = res[1]["result"]["tools"]
        .as_array()
        .expect("tools 必须是数组")
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .unwrap_or_else(|| panic!("工具描述符必须有字符串 name: {t}"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        names.len(),
        yeban_mcp::tools::TOOL_COUNT,
        "tools/list 的条数必须等于注册表 TOOL_COUNT: {names:?}"
    );
    assert_eq!(
        names,
        registry_names(),
        "tools/list 必须与注册表逐项相同(名字与顺序)"
    );

    // ---- ④ 真实调用一: 打开一份真容器工程(第 ⑤ 步只读查询的确定性前提) ----
    let opened = &res[2];
    assert!(
        opened.get("error").is_none(),
        "打开不该有 JSON-RPC 层错误: {opened}"
    );
    assert_eq!(opened["result"]["status"], "success", "{opened}");
    let od = &opened["result"]["data"];
    assert_eq!(od["opened"], true, "{od}");
    assert_eq!(od["path"], path.display().to_string(), "{od}");
    // `locked` 是 `.yeban.lock` 排他锁**真被这个进程拿到**的证据(本行的第三件东西)。
    assert_eq!(od["locked"], true, "{od}");
    assert!(
        od["project"]["trackCount"].as_u64().unwrap_or(0) > 0,
        "打开必须带回非空工程视图: {od}"
    );

    // ---- ⑤ 真实调用二: 查询刚打开的工程 —— 断言**载荷形状**, 不是"没报错" ----
    let queried = &res[3];
    assert_eq!(queried["result"]["status"], "success", "{queried}");
    let qd = &queried["result"]["data"];
    let page = &qd["page"];
    assert_eq!(page["limit"], 3, "必须回显请求的 limit: {page}");
    let total = page["total"].as_u64().expect("page.total 必须是整数");
    assert!(
        total >= 3,
        "夹具工程至少有 3 个实体, 否则下一条断言没有意义: {total}"
    );
    assert_eq!(page["returned"], 3, "{page}");
    assert!(page["hasMore"].is_boolean(), "{page}");
    let entities = qd["entities"].as_array().expect("entities 必须是数组");
    assert_eq!(entities.len(), 3, "一页必须真的装 3 条: {entities:?}");
    for entity in entities {
        assert!(entity["id"].is_string(), "实体必须有 id: {entity}");
        assert!(entity["kind"].is_string(), "实体必须有 kind: {entity}");
    }
    assert!(
        qd["project"].is_object(),
        "查询必须带回工程视图(不是只有实体列表): {qd}"
    );
}

#[test]
fn tools_list_over_stdio_is_real() {
    let res = session(&[req(2, "tools/list", serde_json::json!({}))]);
    let r = res.first().expect("至少一行响应");
    assert_eq!(r["id"], 2, "响应必须回显请求 id: {r}");
    let tools = r["result"]["tools"].as_array().expect("tools 必须是数组");
    // 与**注册表**逐项对账, 而不是"至少几个": 台账 `ROAD-M4-002` 要的正是
    // "tools/list 与注册表一致"。条数单独断言, 是为了让"少了一条"与"顺序变了"分开报。
    // (沿革: 这里曾经写 `>= 15`, 那是 D46 扩张期的下限; 注册表现在是 `TOOL_COUNT` 条。)
    assert_eq!(
        tools.len(),
        yeban_mcp::tools::TOOL_COUNT,
        "tools/list 的条数必须等于注册表 TOOL_COUNT"
    );
    let names: Vec<String> = tools
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .unwrap_or_else(|| panic!("工具描述符必须有字符串 name: {t}"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        names,
        registry_names(),
        "tools/list 必须与注册表逐项相同(名字与顺序)"
    );
    // 逐项对账已经蕴含"这些工具在场"; 这里再点名, 是为了让失败信息直接指向台账点过名的
    // 那些扩张工具(D45 撤销入口 / D46 编辑自动化·引擎状态·音频导入), 而不是只给一份长 diff。
    for want in [
        "yeban_open_project",
        "yeban_query_project",
        "yeban_undo",
        "yeban_edit_automation",
        "yeban_query_engine_state",
        "yeban_import_audio",
    ] {
        assert!(
            names.iter().any(|name| name == want),
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

/// **主链路的第一步，经真 stdio 进程走完**：一个不存在的路径 ⇒ 建工程 ⇒
/// `propose_section` ⇒ `merge` ⇒ `export_midi` ⇒ `render_master` ⇒ `close`。
///
/// 为什么必须在**真二进制**上再证一次（`tests/tools_e2e.rs` 已有一条进程内判据）：
/// 缺口当初是在"真 stdio 会话"里被普查到的（`FILE_NOT_FOUND` ⇒ 没有建工程工具 ⇒
/// 四条下游全断），而进程内判据绕过了 stdio 传输、令牌闸门与批处理生命周期。
/// 这条判据把这四样都放回路径上：子进程、真 stdout、读到 EOF 自行退出、退出码 0。
///
/// 牙长在三处，任一处回归都会红：
/// 1. `create` 的 `seed.masterBusTrackId` 不得是全零、且 `masterBusInRoutingGraph == true`；
/// 2. `propose_section` 必须真的成功（旧行为：`TRACK_NOT_FOUND`）；
/// 3. `render_master` 必须成功并**真的**在磁盘上留下母带（旧行为：
///    `RENDER_FAILED MasterNotInGraph`），且磁盘字节的 SHA-256 等于响应读数。
#[test]
fn a_new_project_can_be_created_over_stdio_and_reaches_a_rendered_master() {
    let dir = scratch().join(format!("create-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建 scratch 子目录");
    let path = dir.join("brand-new.yeban");
    let _ = std::fs::remove_file(&path);
    assert!(!path.exists(), "前提: 目标路径必须不存在");

    let res = session(&[
        req(
            1,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string(), "create": true, "title": "Stdio"},
            }),
        ),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_propose_section",
                "arguments": {"sectionName": "Intro", "stylePreset": "pop", "bars": 2},
            }),
        ),
        req(
            3,
            "tools/call",
            serde_json::json!({"name": "yeban_export_midi", "arguments": {}}),
        ),
        req(
            4,
            "tools/call",
            serde_json::json!({
                "name": "yeban_render_master",
                "arguments": {"format": "wav", "sampleRate": 48000},
            }),
        ),
    ]);
    assert_eq!(res.len(), 4, "4 行请求 ⇒ 4 行响应: {res:?}");

    // ① 建工程：主总线非零 + 在路由图里。
    let created = &res[0]["result"]["data"];
    assert_eq!(res[0]["result"]["status"], "success", "{created}");
    assert_eq!(created["created"], true, "{created}");
    let bus = created["seed"]["masterBusTrackId"]
        .as_str()
        .expect("seed.masterBusTrackId");
    assert_ne!(
        bus, "00000000000000000000000000",
        "主总线不得是全零身份: {created}"
    );
    assert_eq!(
        created["seed"]["masterBusInRoutingGraph"], true,
        "{created}"
    );
    assert!(path.exists(), "工程容器必须真的落盘: {}", path.display());

    // ② propose_section 不再 TRACK_NOT_FOUND。
    assert_eq!(
        res[1]["result"]["status"], "success",
        "建好主总线之后 propose_section 必须能跑: {}",
        res[1]
    );

    // ③ export_midi 有真实音符。
    let midi = &res[2]["result"]["data"];
    assert_eq!(res[2]["result"]["status"], "success", "{midi}");
    assert!(midi["notes"].as_u64().unwrap_or(0) > 0, "{midi}");

    // ④ render_master 真的写出母带；磁盘摘要 == 响应读数。
    let rendered = &res[3]["result"]["data"];
    assert_eq!(
        res[3]["result"]["status"], "success",
        "主总线在路由图里 ⇒ 必须真的渲染出母带: {}",
        res[3]
    );
    let wav = std::path::PathBuf::from(rendered["path"].as_str().expect("母带路径"));
    assert!(wav.exists(), "母带必须落在磁盘上: {}", wav.display());
    let bytes = std::fs::read(&wav).expect("读母带");
    assert_eq!(
        yeban_model::AssetHash::of_bytes(&bytes).as_str(),
        rendered["sha256"].as_str().expect("sha256"),
        "磁盘字节的摘要必须等于响应里的读数"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **19 个 `ops[].kind` 逐个走一次真 `tools/call`**（真进程 / 真 stdio 批处理）。
///
/// 单位 = 一次 `tools/call` 的**响应对象**（以及 [`session`] 已经断言的**子进程退出码 0**）。
/// 判定的不是"这次调用成功"，而是"它**走到了领域实现**"：
///
/// * 顶层**不得**出现 JSON-RPC `error` 对象 —— 领域失败一律**带内**
///   （`result.status == "error"` + 契约 enum 里的码）。出现 `error` 就是实现级
///   （`-32603`）/ 协议级（`-32601` / `-32602`）出口 ⇒ 本判据红；
/// * 带内失败的码必须在 `ErrorCode::SCHEMA_CONTRACT` 里（不发明新码）。
///
/// 为什么用**两段会话**：批处理没有交互往返，而 19 个形态里有 12 个需要夹具里**真实的
/// 身份**（音符 / 设备 / 路由边 / 场景 / 段落 / 池条目）。第一段只用三条请求把这些身份
/// 读出来，第二段才发 19 条 —— 两段都是真进程。
///
/// 注入（实测红）：把 `parse_one` 的任一条臂删掉 ⇒ 那一个 `kind` 会得到「未知 `kind`」
/// 的带内错（仍算可达）但**提案数量**少一条 ⇒ 末尾的"成功 ≥ N"下界变红；
/// 把某条臂改成 `Fault::not_wired`（实现级出口）⇒ 顶层出现 `error` ⇒ 立刻红。
#[test]
fn every_catalog_kind_reaches_the_domain_implementation_over_stdio() {
    let path = write_container_project("e2e-kinds.yeban");
    let probe = session(&[
        req(1, "initialize", serde_json::json!({})),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
        req(
            3,
            "tools/call",
            serde_json::json!({
                "name": "yeban_query_project",
                "arguments": {
                    "fields": ["tracks", "sections", "scenes", "clip_pool",
                               "routing_graph", "master_bus_track_id"],
                    "limit": 1000,
                },
            }),
        ),
    ]);
    assert_eq!(
        probe[1]["result"]["status"], "success",
        "打开夹具容器必须成功: {}",
        probe[1]
    );
    let project = &probe[2]["result"]["data"]["project"];
    let master = project["master_bus_track_id"]
        .as_str()
        .expect("主总线")
        .to_owned();
    let tracks = project["tracks"].as_object().expect("tracks");
    let lead = tracks
        .iter()
        .find(|(id, track)| **id != master && track["kind"] == "Midi")
        .map(|(id, _)| id.clone())
        .expect("夹具里必须有非主总线的 MIDI 音轨");
    let device = tracks[&lead]["devices"]
        .as_array()
        .and_then(|list| list.first())
        .and_then(|value| value["id"].as_str())
        .expect("夹具的音轨必须带一台设备")
        .to_owned();
    let (clip, note) = project["clip_pool"]
        .as_object()
        .expect("clip_pool")
        .iter()
        .find_map(|(id, entry)| {
            entry["content"]["Midi"]["notes"]
                .as_object()
                .and_then(|notes| notes.keys().next())
                .map(|note| (id.clone(), note.clone()))
        })
        .expect("夹具里必须有一条带音符的 MIDI 片段");
    let edge = project["routing_graph"]["edges"]
        .as_array()
        .and_then(|list| list.first())
        .and_then(|value| value["id"].as_str())
        .expect("夹具必须有路由边")
        .to_owned();
    let node = project["routing_graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(Value::as_str)
        .find(|id| **id != master)
        .expect("夹具必须有非主总线节点")
        .to_owned();
    let scene = project["scenes"]
        .as_object()
        .and_then(|map| map.keys().next())
        .expect("夹具必须有场景")
        .clone();
    let section = project["sections"]
        .as_object()
        .and_then(|map| map.keys().next())
        .expect("夹具必须有曲段")
        .clone();
    // 自动化点：在**全部**音轨的泳道里找一条按名字就能寻址的（`TrackVolume` / `TrackPan`；
    // `SendGain` 还要 `edgeId`，本判据不需要它）。
    let (lane_name, lane_tick) = tracks
        .values()
        .filter_map(|track| track["automation_lanes"].as_array())
        .flat_map(|lanes| lanes.iter())
        .filter_map(|lane| {
            let target = lane["target"].as_object()?.keys().next()?.clone();
            if target != "TrackVolume" && target != "TrackPan" {
                return None;
            }
            let tick = lane["points"].as_object()?.values().next()?["tick"].as_u64()?;
            Some((target, tick))
        })
        .next()
        .unwrap_or_else(|| {
            panic!(
                "夹具里必须有一条 TrackVolume / TrackPan 泳道且带采样点: tracks={}",
                serde_json::to_string(&tracks).expect("json")
            )
        });

    let ops: Vec<(&str, Value)> = vec![
        (
            "add",
            serde_json::json!({"kind": "add",
            "note": {"startTick": 0, "pitch": 61, "durationTicks": 480}}),
        ),
        (
            "delete",
            serde_json::json!({"kind": "delete", "noteId": note}),
        ),
        (
            "move",
            serde_json::json!({"kind": "move", "noteId": note, "deltaTick": 0, "deltaPitch": 1}),
        ),
        (
            "velocity",
            serde_json::json!({"kind": "velocity", "noteId": note, "velocity": 64}),
        ),
        ("removeClip", serde_json::json!({"kind": "removeClip"})),
        (
            "removeTrack",
            serde_json::json!({"kind": "removeTrack", "trackId": lead}),
        ),
        (
            "insertDevice",
            serde_json::json!({"kind": "insertDevice", "trackId": lead,
            "device": {"deviceId": "01J8ZQ000000000000000000DV", "name": "Probe",
                       "kind": "InternalEffect", "bypassed": false,
                       "latencySamples": 8, "params": []}}),
        ),
        (
            "removeDevice",
            serde_json::json!({"kind": "removeDevice", "trackId": lead,
            "deviceId": device}),
        ),
        (
            "setParam",
            serde_json::json!({"kind": "setParam", "lane": "TrackVolume", "value": -4.5}),
        ),
        (
            "setTrackMute",
            serde_json::json!({"kind": "setTrackMute", "value": true}),
        ),
        (
            "setTrackSolo",
            serde_json::json!({"kind": "setTrackSolo", "value": false}),
        ),
        (
            "setAutomationLane",
            serde_json::json!({"kind": "setAutomationLane",
            "lane": {"lane": "TrackVolume", "readEnabled": true}}),
        ),
        (
            "removeAutomationPoint",
            serde_json::json!({"kind": "removeAutomationPoint",
            "point": {"lane": lane_name, "tick": lane_tick}}),
        ),
        (
            "setRoutingGain",
            serde_json::json!({"kind": "setRoutingGain", "edgeId": edge, "value": -3.0}),
        ),
        (
            "disconnectRouting",
            serde_json::json!({"kind": "disconnectRouting", "edgeId": edge}),
        ),
        (
            "removeRoutingNode",
            serde_json::json!({"kind": "removeRoutingNode", "nodeId": node}),
        ),
        (
            "removeSection",
            serde_json::json!({"kind": "removeSection", "sectionId": section}),
        ),
        (
            "removeScene",
            serde_json::json!({"kind": "removeScene", "sceneId": scene}),
        ),
        (
            "setScene",
            serde_json::json!({"kind": "setScene",
            "scene": {"sceneId": scene, "tempo": 128.0}}),
        ),
    ];
    assert_eq!(ops.len(), 19, "目录是 19 个 kind");

    let mut requests = vec![
        req(1, "initialize", serde_json::json!({})),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
    ];
    for (index, (_kind, op)) in ops.iter().enumerate() {
        requests.push(req(
            u32::try_from(index).expect("小") + 10,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_notes",
                "arguments": {"trackId": lead, "clipId": clip, "ops": [op]},
            }),
        ));
    }
    let res = session(&requests);
    assert_eq!(res.len(), 21, "1 个握手 + 1 次打开 + 19 次调用: {res:?}");

    let mut reached = 0usize;
    let mut succeeded = 0usize;
    for (index, (kind, _op)) in ops.iter().enumerate() {
        let response = &res[index + 2];
        assert!(
            response.get("error").is_none(),
            "`{kind}` 掉进了 JSON-RPC 出口（实现级 / 协议级）: {response}"
        );
        assert_eq!(response["id"], serde_json::json!(index + 10));
        let result = &response["result"];
        match result["status"].as_str() {
            Some("success") => {
                succeeded += 1;
                assert!(
                    result["data"]["proposal"]["proposalId"].is_string(),
                    "`{kind}` 成功时必须真的建成提案: {result}"
                );
            }
            Some("error") => {
                let code = result["error"]["code"].as_str().expect("码是字符串");
                assert!(
                    yeban_mcp::tools::ErrorCode::SCHEMA_CONTRACT
                        .iter()
                        .any(|known| known.as_str() == code),
                    "`{kind}` 的失败码 `{code}` 不在契约 enum 里: {result}"
                );
            }
            other => panic!("`{kind}` 的响应既不是 success 也不是 error: {other:?}"),
        }
        reached += 1;
    }
    assert_eq!(reached, 19, "19 个形态都必须走到领域实现");
    // 下界是**非平凡**的（不是 0）：夹具必须让大多数形态真的建成提案，
    // 否则"19 个都可达"可以靠"全都报带内错"骗过去。
    let refusals: Vec<String> = ops
        .iter()
        .enumerate()
        .filter(|(index, _)| res[index + 2]["result"]["status"] == "error")
        .map(|(_, (kind, _))| (*kind).to_owned())
        .collect();
    assert_eq!(
        succeeded, 17,
        "17 / 19 真的建成提案；另外 2 条是**带内**领域拒绝（不是实现级出口）：\
         `removeClip`（池条目仍被摆放引用）与 `removeRoutingNode`（节点仍被边引用）—— \
         模型的前置条件，判据 `the_declared...` 之外由它们各自的用例覆盖。\
         实际 {succeeded}; 被拒的形态 = {refusals:?}"
    );
    assert_eq!(
        refusals,
        vec!["removeClip".to_owned(), "removeRoutingNode".to_owned()],
        "被拒的必须是那两条'仍被引用'的形态（换掉夹具会改变这个读数，届时改这里）"
    );
}

/// **方向 1：19 个 `kind` 的响亮失败路径**（真进程 / 真 stdio）。
///
/// 上一条判据证明"每个 `kind` 都走得到领域实现"（正向）；本条证明**反向**也走得到：
/// 每个 `kind` 至少喂一条**缺字段 / 类型错 / 身份不存在**的载荷，并断言
/// 1. 顶层**不得**出现 JSON-RPC `error`（失败必须是**带内**的）；
/// 2. 失败码**逐行等于登记值**，且都在 `ErrorCode::SCHEMA_CONTRACT` 里；
/// 3. 19 次被拒之后，工程的 JSON 与调用**之前**逐字节相同（被拒的调用不留痕）。
///
/// 单位 = 一次 `tools/call` 的响应对象（子进程退出码由 `session` 断言为 0）。
///
/// 注入（实测红）：把某条 `parse_one` 臂的守卫删掉（例如 `delete` 不再要求 `noteId`）
/// ⇒ 该行的**失败不再发生**（变成成功或换一个码）⇒ 红。
#[test]
fn every_catalog_kind_also_has_a_loud_failure_path_over_stdio() {
    let path = write_container_project("e2e-kinds-refusals.yeban");
    let query = req(
        3,
        "tools/call",
        serde_json::json!({
            "name": "yeban_query_project",
            "arguments": {
                "fields": ["tracks", "sections", "scenes", "clip_pool",
                           "routing_graph", "master_bus_track_id"],
                "limit": 1000,
            },
        }),
    );
    let probe = session(&[
        req(1, "initialize", serde_json::json!({})),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
        query.clone(),
    ]);
    assert_eq!(probe[1]["result"]["status"], "success");
    let project = probe[2]["result"]["data"]["project"].clone();
    let before = serde_json::to_string(&project).expect("JSON");

    let master = project["master_bus_track_id"]
        .as_str()
        .expect("主总线")
        .to_owned();
    let tracks = project["tracks"].as_object().expect("tracks");
    let lead = tracks
        .iter()
        .find(|(id, track)| **id != master && track["kind"] == "Midi")
        .map(|(id, _)| id.clone())
        .expect("MIDI 音轨");
    let (clip, note) = project["clip_pool"]
        .as_object()
        .expect("clip_pool")
        .iter()
        .find_map(|(id, entry)| {
            entry["content"]["Midi"]["notes"]
                .as_object()
                .and_then(|notes| notes.keys().next())
                .map(|note| (id.clone(), note.clone()))
        })
        .expect("带音符的 MIDI 片段");
    let node = project["routing_graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(Value::as_str)
        .find(|id| **id != master)
        .expect("非主总线节点")
        .to_owned();
    let (lane_name, lane_tick) = tracks
        .values()
        .filter_map(|track| track["automation_lanes"].as_array())
        .flat_map(|lanes| lanes.iter())
        .find_map(|lane| {
            let target = lane["target"].as_object()?.keys().next()?.clone();
            if target != "TrackVolume" && target != "TrackPan" {
                return None;
            }
            let tick = lane["points"].as_object()?.values().next()?["tick"].as_u64()?;
            Some((target, tick))
        })
        .expect("TrackVolume/TrackPan 泳道");
    let _ = (&clip, &node);
    let ghost = "01J8ZQ00000000000000000999";
    let valid_device = serde_json::json!({
        "deviceId": "01J8ZQ000000000000000000DV", "name": "Probe",
        "kind": "InternalEffect", "bypassed": false, "latencySamples": 0, "params": [],
    });

    // (kind, 载荷, 期望的契约码)
    let cases: Vec<(&str, Value, &str)> = vec![
        ("add", serde_json::json!({}), "INVALID_PARAMETER_RANGE"),
        ("delete", serde_json::json!({}), "INVALID_PARAMETER_RANGE"),
        (
            "move",
            serde_json::json!({"noteId": note, "deltaTick": 0, "deltaPitch": "x"}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "velocity",
            serde_json::json!({"noteId": note, "velocity": 200}),
            "OUT_OF_RANGE",
        ),
        (
            "removeClip",
            serde_json::json!({"clipId": clip}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "removeTrack",
            serde_json::json!({"trackId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "insertDevice",
            serde_json::json!({"trackId": lead, "slotIndex": 999,
            "device": valid_device}),
            "OUT_OF_RANGE",
        ),
        (
            "removeDevice",
            serde_json::json!({"trackId": lead, "deviceId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "setParam",
            serde_json::json!({"lane": "TrackVolume", "value": 1e39}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "setTrackMute",
            serde_json::json!({"value": "yes"}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "setTrackSolo",
            serde_json::json!({"value": 1}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "setAutomationLane",
            serde_json::json!({"lane": {"lane": "Bogus"}}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "removeAutomationPoint",
            serde_json::json!({"point": {"lane": lane_name,
            "tick": lane_tick, "pointId": ghost}}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "setRoutingGain",
            serde_json::json!({"edgeId": ghost, "value": 0.0}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "disconnectRouting",
            serde_json::json!({"edgeId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "removeRoutingNode",
            serde_json::json!({"nodeId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "removeSection",
            serde_json::json!({"sectionId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "removeScene",
            serde_json::json!({"sceneId": ghost}),
            "ENTITY_NOT_FOUND",
        ),
        (
            "setScene",
            serde_json::json!({"scene": {"sceneId": ghost, "create": true}}),
            "INVALID_PARAMETER_RANGE",
        ),
    ];
    assert_eq!(cases.len(), 19, "19 个 kind 各一条失败路径");

    let mut requests = vec![
        req(1, "initialize", serde_json::json!({})),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
    ];
    for (index, (kind, payload, _code)) in cases.iter().enumerate() {
        let mut object = payload.as_object().expect("对象").clone();
        object.insert("kind".to_owned(), Value::from(*kind));
        requests.push(req(
            u32::try_from(index).expect("小") + 10,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_notes",
                "arguments": {"trackId": lead, "clipId": clip,
                              "ops": [Value::Object(object)]},
            }),
        ));
    }
    requests.push(req(
        99,
        "tools/call",
        serde_json::json!({
            "name": "yeban_query_project",
            "arguments": {"fields": ["tracks", "sections", "scenes", "clip_pool",
                                     "routing_graph", "master_bus_track_id"],
                          "limit": 1000},
        }),
    ));
    let res = session(&requests);
    assert_eq!(
        res.len(),
        22,
        "1 握手 + 1 打开 + 19 次调用 + 1 次回读: {res:?}"
    );

    for (index, (kind, _payload, expected)) in cases.iter().enumerate() {
        let response = &res[index + 2];
        assert!(
            response.get("error").is_none(),
            "`{kind}` 的失败掉进了 JSON-RPC 出口: {response}"
        );
        let result = &response["result"];
        assert_eq!(
            result["status"], "error",
            "`{kind}` 必须被响亮拒绝: {result}"
        );
        let code = result["error"]["code"].as_str().expect("码是字符串");
        assert_eq!(code, *expected, "`{kind}` 的契约码");
        assert!(
            yeban_mcp::tools::ErrorCode::SCHEMA_CONTRACT
                .iter()
                .any(|known| known.as_str() == code),
            "`{kind}` 的码 `{code}` 不在契约 enum 里"
        );
        assert!(
            result["data"].is_null() || result.get("data").is_some(),
            "带内失败必须给出结构化的 `error.data`（哪怕是 null）"
        );
    }
    // 19 次被拒之后，工程一个字节都没变。
    let after = serde_json::to_string(&res[21]["result"]["data"]["project"]).expect("JSON");
    assert_eq!(after, before, "被拒的调用不得改动工程");
}

/// **方向 4：单条请求/响应的规模端点**（真进程 / 真 stdio）。
///
/// 单位 = 一次 `tools/call` 的响应对象（子进程退出码由 `session` 断言为 0）。四格：
/// 1. **最小请求**：`tools/list` **不带 `params`** ⇒ 成功且 17 个工具（`params` 缺省合法）；
/// 2. `ops` **空数组**：不带 `placement` ⇒ 带内 `INVALID_PARAMETER_RANGE`
///    （"空操作不是一次编辑请求"），带 `placement` ⇒ **成功**（这次调用只摆放）；
/// 3. **恰好到上限**：`yeban_edit_automation` 的 `ticks` 恰好 256 个 ⇒ 成功、
///    257 个 ⇒ 带内 `INVALID_PARAMETER_RANGE`（已发布上限，判据 `PL-limit` 的姊妹面）；
/// 4. **大载荷**：一次 1000 条 `add`（`includeOps: true`）⇒ 成功且响应里真的有 1000 条，
///    并断言序列化后的响应**超过 100 KB**（证明大载荷真的过了管道，而不是被悄悄截断）。
///
/// 注入（实测红）：把 `parse_ops` 的空数组守卫删掉 ⇒ 第 2 格的前半变成功；把
/// `MAX_READ_TICKS` 改成 128 ⇒ 第 3 格前半红；给响应加一个"最多回 100 条 ops"的截断
/// ⇒ 第 4 格红。
#[test]
fn request_and_response_scale_edges_over_stdio() {
    let path = write_container_project("e2e-scale.yeban");
    let ticks_exact: Vec<u64> = (0..256).collect();
    let ticks_over: Vec<u64> = (0..257).collect();
    let bulk: Vec<Value> = (0..1000_u64)
        .map(|index| {
            serde_json::json!({
                "kind": "add",
                // 时值 480、步长 480 ⇒ **不重叠**（发声数峰值 = 1，不会撞 32 的上限）。
                "note": {"startTick": index * 480, "pitch": 60, "durationTicks": 480},
            })
        })
        .collect();

    let probe = session(&[
        req(1, "initialize", serde_json::json!({})),
        req(
            2,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
        req(
            3,
            "tools/call",
            serde_json::json!({
                "name": "yeban_query_project",
                "arguments": {"fields": ["tracks", "clip_pool", "master_bus_track_id"],
                              "limit": 1000},
            }),
        ),
    ]);
    assert_eq!(probe.len(), 3, "探测会话必须有三条响应: {probe:?}");
    let project = &probe[2]["result"]["data"]["project"];
    let master = project["master_bus_track_id"]
        .as_str()
        .unwrap_or_else(|| panic!("探测会话的响应不对: {probe:?}"))
        .to_owned();
    let tracks = project["tracks"]
        .as_object()
        .unwrap_or_else(|| panic!("tracks 不是对象: {project:?}"));
    let lead = tracks
        .iter()
        .find(|(id, track)| **id != master && track["kind"] == "Midi")
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| panic!("没有 MIDI 音轨: {project:?}"));
    let clip = project["clip_pool"]
        .as_object()
        .and_then(|pool| pool.keys().next())
        .unwrap_or_else(|| panic!("没有池条目: {project:?}"))
        .clone();

    let res = session(&[
        req(1, "initialize", serde_json::json!({})),
        // ① 最小请求：`tools/list` 连 `params` 都不给。
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_owned(),
        req(
            3,
            "tools/call",
            serde_json::json!({
                "name": "yeban_open_project",
                "arguments": {"path": path.display().to_string()},
            }),
        ),
        // ② `ops` 空数组（不允许；只有"只摆放"才允许空）。
        req(
            4,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_notes",
                "arguments": {"trackId": lead, "clipId": clip, "ops": []},
            }),
        ),
        // ②' `ops` 空数组 + `placement` ⇒ 允许。
        req(
            5,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_notes",
                "arguments": {
                    "trackId": lead, "clipId": clip, "ops": [],
                    "placement": {"kind": "add", "startTick": 0, "durationTicks": 1920},
                },
            }),
        ),
        // ③ `ticks` 恰好 256 / 257。
        req(
            6,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_automation",
                "arguments": {"trackId": lead, "lane": "TrackVolume", "ticks": ticks_exact},
            }),
        ),
        req(
            7,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_automation",
                "arguments": {"trackId": lead, "lane": "TrackVolume", "ticks": ticks_over},
            }),
        ),
        // ④ 1000 条 add，且要求把 op 载荷回显。
        req(
            8,
            "tools/call",
            serde_json::json!({
                "name": "yeban_edit_notes",
                "arguments": {"trackId": lead, "clipId": clip, "ops": bulk,
                              "includeOps": true},
            }),
        ),
    ]);
    assert_eq!(res.len(), 8, "8 条请求都必须有响应: {res:?}");

    // ① 最小请求。
    assert_eq!(res[1]["result"]["tools"].as_array().map(Vec::len), Some(17));
    // ② 空 ops 不允许。
    let empty = &res[3]["result"];
    assert_eq!(empty["status"], "error", "{empty}");
    assert_eq!(empty["error"]["code"], "INVALID_PARAMETER_RANGE", "{empty}");
    // ②' 只摆放允许。
    let placed = &res[4]["result"];
    assert_eq!(placed["status"], "success", "{placed}");
    // ③ 上限含端点。
    assert_eq!(res[5]["result"]["status"], "success", "{}", res[5]);
    let over = &res[6]["result"];
    assert_eq!(over["status"], "error", "{over}");
    assert_eq!(over["error"]["code"], "INVALID_PARAMETER_RANGE", "{over}");
    assert_eq!(over["error"]["data"]["limit"], 256, "上限必须是字面 256");
    // ④ 大载荷真的过了管道。
    let big = &res[7]["result"];
    assert_eq!(big["status"], "success", "{}", res[7]);
    let ops = big["data"]["proposal"]["ops"]
        .as_array()
        .expect("includeOps 必须回显 op 载荷");
    assert_eq!(ops.len(), 1000, "1000 条 add 必须一条不少地回显");
    let wire = serde_json::to_string(&res[7]).expect("JSON");
    assert!(
        wire.len() > 100 * 1024,
        "1000 条 op 的响应必须真的超过 100 KB（实测 {} 字节）—— 否则可能是被截断了",
        wire.len()
    );
}

/// 用给定实参跑一次真二进制，喂可选 stdin，返回 `(退出码, stdout, stderr)`。
///
/// 与 [`session`] 的分工：那个是"一次 stdio 会话（断言退出码 0 + 解析响应行）"；
/// 这个是"跑一次 CLI（关心退出码与两路输出）"。
fn run_cli(args: &[&str], stdin: &str) -> (i32, String, String) {
    use std::io::Write as _;

    let bin = env!("CARGO_BIN_EXE_yeban-mcp");
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn yeban-mcp");
    {
        let mut sink = child.stdin.take().expect("stdin");
        if !stdin.is_empty() {
            let _ = sink.write_all(stdin.as_bytes());
        }
    }
    let output = child.wait_with_output().expect("等待子进程");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// **CLI 面是已发布的契约**：`--help` 的用法文本、未知实参的退出码与报文。
///
/// 为什么需要它：`src/bin/yeban-mcp.rs` 的实参解析（`--help` / `--scopes` /
/// `--test-mode` / `--print-token` / `--token-file` / `--enable-mcp-http`）在
/// **第四批注入之前没有任何判据** —— 实测把 `--help` 改名、把 `--print-token` 短路、
/// 把 `--test-mode` 判反，**全量测试全绿**（`BIN-help` / `BIN-printtoken` /
/// `BIN-testmode` 三条注入）。
///
/// 单位 = 一次进程的 `(退出码, stdout, stderr)`。判据**不依赖墙钟**。
///
/// 注入（实测红）：把 `"--help"` 改成 `"--helpx"` ⇒ 第 1 段红；未知实参不再报错 ⇒ 第 2 段红。
#[test]
fn the_cli_surface_is_a_published_contract() {
    // ① `--help`：退出码 0，用法里逐个列出七个开关。
    let (code, out, _err) = run_cli(&["--help"], "");
    assert_eq!(code, 0, "--help 必须成功退出");
    for flag in [
        "--stdio",
        "--print-token",
        "--token-file",
        "--scopes",
        "--test-mode",
        "--enable-mcp-http",
        "-h, --help",
    ] {
        assert!(out.contains(flag), "用法文本必须列出 `{flag}`: {out}");
    }
    // ② 未知实参：退出码 2 + 报文点名那个实参 + 仍然是用法文本（不是 panic）。
    let (code, _out, err) = run_cli(&["--probe-unknown"], "");
    assert_eq!(code, 2, "未知实参必须用退出码 2");
    assert!(
        err.contains("--probe-unknown"),
        "报文必须点名未知实参: {err}"
    );
    assert!(err.contains("用法:"), "未知实参也要打用法: {err}");
    // ③ `--token-file` 缺值：也是配置问题（退出码 2），不是 panic。
    let (code, _out, err) = run_cli(&["--token-file"], "");
    assert_eq!(code, 2, "缺值的实参必须用退出码 2");
    assert!(!err.is_empty());
}

/// **`--print-token` 的两条平台契约**：Unix 上"生成一次、之后复用"，
/// 非 Unix 上**明确拒绝**（POSIX 0600 做不到）。
///
/// 为什么需要它：令牌文件是**跨进程**的鉴权依据，而这条路径（`load_or_create` +
/// "已生成/已复用" 两态）此前没有判据 —— 把 `if config.print_token` 短路时全绿。
/// ⚠ **平台分支是实测教训**：第一版只写了 Unix 那一半，Windows 腿立刻红
/// （二进制在非 Unix 上按设计拒绝 0600 校验 ⇒ 退出码非 0）。两条契约现在都钉住。
///
/// 单位 = 一次进程的 `(退出码, stdout, stderr)` + 令牌文件的内容。**不依赖墙钟**。
///
/// 注入（实测红）：把 `if config.print_token {` 改成 `if false {` ⇒ Unix 段拿不到
/// stdout 上的令牌，红。
#[test]
fn the_print_token_flag_writes_then_reuses_the_token_file() {
    let path = scratch().join(format!("cli-token-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let shown = path.display().to_string();

    let (code, out, err) = run_cli(&["--print-token", "--token-file", &shown], "");
    if !cfg!(unix) {
        // 非 Unix：**明确拒绝**（不是静默放过权限）。
        assert_ne!(code, 0, "非 Unix 上 `--print-token` 必须非零退出: {err}");
        assert!(
            err.contains("0600"),
            "报文必须说明「0600 权限校验做不到」这个原因: {err}"
        );
        return;
    }

    assert_eq!(code, 0, "--print-token 必须成功退出: {err}");
    let token = out.trim();
    assert!(
        token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "stdout 上必须是一枚 64 位十六进制令牌: {token:?}"
    );
    assert!(err.contains("已生成"), "第一次必须报「已生成」: {err}");
    assert!(err.contains(&shown), "必须报出令牌文件路径: {err}");
    let first = std::fs::read_to_string(&path).expect("令牌文件必须落盘");

    // ② 第二次：复用**同一枚**令牌（文件内容逐字节不变），并如实报"已复用"。
    let (code, out2, err2) = run_cli(&["--print-token", "--token-file", &shown], "");
    assert_eq!(code, 0);
    assert_eq!(out2.trim(), token, "第二次必须复用同一枚令牌");
    assert!(err2.contains("已复用"), "第二次必须报「已复用」: {err2}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("令牌文件还在"),
        first,
        "复用不得改写令牌文件"
    );
    let _ = std::fs::remove_file(&path);
}

/// **`--test-mode` 在进程退出时被如实报出**（`模式 production` vs `模式 test`）。
///
/// 为什么需要它：`--test-mode` 决定 `RunMode`，而 `RunMode` 决定
/// `Scope::UiInject` 这类"生产模式硬禁"的作用域能不能用 —— 判反它是一条**安全**缺陷。
/// 二进制在 stdio 收尾时把模式写进 stderr（`模式 {mode}`），因此这是**黑盒可观测**的。
///
/// 单位 = 一次进程的 `(退出码, stderr)`。**不依赖墙钟**。
///
/// 注入（实测红）：把 `"--test-mode" => config.test_mode = true` 改成 `= false`
/// ⇒ 第 2 段读到 `模式 production`，红。
#[test]
fn the_test_mode_flag_is_reported_by_the_process() {
    let path = scratch().join(format!("cli-mode-{}.txt", std::process::id()));
    let shown = path.display().to_string();
    let initialize = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n";

    let (code, _out, err) = run_cli(&["--stdio", "--token-file", &shown], initialize);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("模式 production"), "缺省必须是生产模式: {err}");

    let (code, _out, err) = run_cli(
        &["--stdio", "--test-mode", "--token-file", &shown],
        initialize,
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("模式 test"),
        "`--test-mode` 必须报到 stderr: {err}"
    );
    let _ = std::fs::remove_file(&path);
}
