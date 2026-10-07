//! `ADR-0001` **D46** 的三类扩展工具（自动化泳道 / 设备与引擎 / 音频导入）的**承重判据**。
//!
//! ## 覆盖的判据（①..⑧，逐条对应任务书）
//!
//! | # | 判据 | 本文件的用例 |
//! | :--- | :--- | :--- |
//! | ① | 每个新工具**真做事**（读类：改模型 ⇒ 读数变；写类：工程真的变了 + 逆操作逐字节回退） | `automation_read_follows_the_model_through_the_unique_entry`、`automation_write_lands_in_the_project_and_undo_restores_byte_for_byte`、`import_audio_registers_a_clip_and_undo_restores_byte_for_byte`、`engine_state_reads_three_single_sources` |
//! | ① | **来源标签如实**：两个直接编辑工具的落盘作者是 `OpOrigin::McpEdit`（不再是借来的 `AutomationRecord` / `Import`），且响应的来源块点名新变体 | `direct_edits_are_authored_by_the_mcp_edit_origin_in_the_persisted_log`、`direct_edit_origins_in_production_sources_are_the_mcp_edit_variant` |
//! | ② | `dryRun=true` ⇒ 状态一位不变 + 预览与真做一致 | `dry_run_leaves_every_state_bit_identical`、`dry_run_preview_equals_the_real_call_for_all_three_tools` |
//! | ③ | 未知/坏参数 ⇒ **既有**错误码（不发明新码） | `bad_and_unknown_parameters_only_use_codes_inside_d25`、`error_code_vocabulary_is_the_contract_enum_exactly` |
//! | ④ | 幂等键：同键重复调用不重复生效 | `the_same_idempotency_key_never_applies_twice` |
//! | ⑤ | 新工具在 `tools/list` 与契约样本里可发现 | `the_three_new_tools_are_discoverable` |
//! | ⑥ | 不许出现第二份实现（自动化求值必须走 `automation_value_at`） | `no_second_automation_evaluation_in_production_sources`、`the_write_paths_commit_through_the_single_undo_entry`、`dry_run_entry_points_take_shared_references_only` |
//! | ⑦ | 与既有十工具不冲突 | `the_documented_ten_tools_are_untouched`（契约侧）+ 既有 6 个测试文件原样通过 |
//! | ⑧ | 门禁 | `cargo fmt` / `run-gates.sh light`（本机）+ CI（重活） |
//! | ⑨ | `[D56]` 诊断包：响应 `projectIncluded` 与**包内字节**不许互相矛盾（隐私默认有牙） | `diagnostics_bundle_content_matches_the_project_included_flag` |
//!
//! ## 为什么还有"文本级守卫"这一半
//!
//! `yeban-mcp` 传递依赖 rayon / symphonia / rubato ⇒ **本机不编译它**。运行期判据只能在
//! CI 上跑，那"本机做注入实验看它变红"就做不到。因此把三条**能从源码文本判定**的性质
//! 抽成零依赖纯函数（`domain::automation_audit` / `domain::extension_audit`），
//! 本文件把它们**也**在真实仓库源码上跑一遍：这样"守卫不空转"这件事在两侧都有证据。
//!
//! ## 本机 vs CI（如实标注）
//!
//! - **本机真跑过**：`rustc --edition 2024 --test` 跑三个零依赖模块（23 条）+ 一个
//!   裸 `rustc` 脚手架把这四条文本守卫跑在**真实源码**上 + 四条注入实验（红→还原→绿）。
//! - **只能 CI 跑**：本文件的全部运行期判据（编译 `yeban-mcp` 需要重依赖）。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::engine_state::EngineReadings;
use yeban_mcp::domain::{automation_audit, extension_audit};
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};
use yeban_mcp::tools::{self, ErrorCode};
use yeban_model::samples::filled_project;
use yeban_model::{
    AssetHash, AutomationPoint, AutomationTarget, ClipContent, CommitGraph, CurveType, EntityId,
    Op, OpOrigin, SampleRate, SessionRuntimeState, YebanProjectV1,
};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个新的分发器（内存工程，不碰文件系统）。
fn dispatcher_with_project(project: YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("fixture"), project, false)
        .expect("注入规范工程");
    dispatcher
}

/// 一个**保证不存在**的工程路径（父目录也不建）。
fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-ext-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `ToolResponse`（`result` 里的 JSON 对象）。
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
    let response = outcome.response.expect("响应");
    response.result.expect("result")
}

/// 一次 `tools/call` 的 **JSON-RPC 层**读数：`(http 状态, JSON-RPC 错误码)`。
///
/// 刻意**只**回这两个标量：判据要断言的是"坏参数走既有的 `-32602`、不是新码"，
/// 而 `jsonrpc::ErrorObject` **没有** `to_value`（`Id::to_value` 才是那个名字）——
/// 与其在这里再拼一份 JSON 形状（那是第 N 份会漂移的表示），不如不要它。
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
    // ⚠ 先把标量取出来：`outcome.response` 被移走之后就不能再对整个 `outcome` 调方法了。
    let status = outcome.http_status;
    let code = outcome.error_code();
    let response = outcome.response.expect("响应");
    // 带内失败（领域失败）不带 JSON-RPC 错误对象；实现级状况才带。
    let looks_like_failure = response.result.is_some() || response.error.is_some();
    assert!(looks_like_failure, "响应必须要么带 result 要么带 error");
    (status, code)
}

/// 工程的**规范化字节**（`undo_session` 的同一口径；逐字节回退判据用它）。
fn project_bytes(dispatcher: &Dispatcher) -> Vec<u8> {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    yeban_mcp::undo_session::canonical_project_bytes(project).expect("规范化字节")
}

/// 从**落盘的** `history.dag` 里读出每条 `StampedOp` 的作者标签与 Op 本体。
///
/// 这是"来源标签"最硬的观测面：读的是真的写进容器的那份 `StampedOp`，
/// 而不是内存里的某个响应字段或一句手抄的字符串。
fn persisted_origins(path: &std::path::Path) -> Vec<(OpOrigin, Op)> {
    let bytes = std::fs::read(path).expect("读容器字节");
    let archive = yeban_model::container::read_project_container(
        &bytes,
        &yeban_model::container::ContainerLimits::default(),
    )
    .expect("容器必须可读");
    let graph: CommitGraph =
        serde_json::from_slice(&archive.history_dag).expect("history.dag 必须是 CommitGraph JSON");
    let mut rows = Vec::new();
    for commit in graph.commits.values() {
        for stamped in &commit.ops {
            rows.push((stamped.origin.clone(), stamped.op.clone()));
        }
    }
    rows
}

/// 样本里那条带设备的音轨（`filled_project` 的 lead）。
fn lead_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| !track.devices.is_empty())
        .expect("样本里必须有带设备的音轨")
        .id
}

/// 一份最小的 16-bit 单声道 WAV。
fn wav_s16(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    wav_with_declared_len(
        sample_rate,
        &samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<u8>>(),
        None,
    )
}

/// 手工 RIFF/WAVE 头；`declared_len` 为 `Some` 时故意与真实数据长度不符
/// （用来构造"头里声称几小时"的输入，从而**真的**撞上 `PcmBudget` 的时长闸门）。
fn wav_with_declared_len(sample_rate: u32, data: &[u8], declared_len: Option<u32>) -> Vec<u8> {
    let declared = declared_len.unwrap_or_else(|| u32::try_from(data.len()).expect("小"));
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36_u32.saturating_add(declared)).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&declared.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// 一份独占临时目录里的音频文件（返回路径；调用方负责清理父目录）。
fn write_audio_fixture(tag: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-mcp-ext-audio-{}-{tag}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建目录");
    let path = dir.join("fixture.wav");
    std::fs::write(&path, bytes).expect("写音频夹具");
    path
}

// ---------------------------------------------------------------------------
// ① / ⑤ 可发现性
// ---------------------------------------------------------------------------

#[test]
fn the_three_new_tools_are_discoverable() {
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
    assert_eq!(
        listed.len(),
        tools::TOOL_COUNT,
        "tools/list 必须列出全部工具"
    );
    for name in [
        "yeban_edit_automation",
        "yeban_query_engine_state",
        "yeban_import_audio",
    ] {
        let descriptor = listed
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("tools/list 里找不到 `{name}`"));
        let spec = tools::tool(name).expect("注册表");
        assert_eq!(descriptor["annotations"]["specId"], spec.spec_id);
        assert_eq!(descriptor["annotations"]["dryRunSupported"], true);
        assert_eq!(descriptor["annotations"]["idempotent"], true);
        assert!(
            descriptor["inputSchema"]["properties"][tools::DRY_RUN_PARAM].is_object(),
            "`{name}` 的 inputSchema 必须声明 dryRun"
        );
        assert!(
            descriptor["inputSchema"]["properties"][tools::IDEMPOTENCY_KEY_PARAM].is_object(),
            "`{name}` 的 inputSchema 必须声明 idempotencyKey"
        );
        assert_eq!(
            descriptor["annotations"]["errorCodes"]
                .as_array()
                .expect("errorCodes")
                .len(),
            spec.errors.len()
        );
    }

    // 契约样本侧：每个新工具一份 `mcp-tools.call.<tool>.json`（导出集合与工具集合双射）。
    let dir = std::env::temp_dir().join(format!("yeban-mcp-ext-samples-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let written = yeban_mcp::samples::export_all(&dir).expect("导出样本");
    let names: Vec<String> = written
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    for name in [
        "yeban_edit_automation",
        "yeban_query_engine_state",
        "yeban_import_audio",
    ] {
        let file = yeban_mcp::samples::call_file(name);
        assert!(names.contains(&file), "缺少契约实例 `{file}`: {names:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ① 读类：改模型 ⇒ 读数跟着变（且走唯一求值入口）
// ---------------------------------------------------------------------------

#[test]
fn automation_read_follows_the_model_through_the_unique_entry() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    let arguments = serde_json::json!({
        "trackId": track.to_canonical_string(),
        "lane": "TrackVolume",
        "ticks": [0, 1920, 3840],
    });
    let first = call_tool(&mut dispatcher, "yeban_edit_automation", arguments.clone());
    assert_eq!(first["status"], "success", "{first}");
    assert_eq!(first["data"]["readOnly"], true, "不给 point ⇒ 只读");
    let values = first["data"]["read"]["values"].as_array().expect("values");
    assert_eq!(values[0]["automationValue"], -6.0);
    assert_eq!(values[1]["automationValue"], -3.0, "tick 1920 = 线性中点");
    assert_eq!(values[2]["automationValue"], 0.0);
    assert_eq!(first["data"]["read"]["entry"], "automation_value_at");
    assert_eq!(first["data"]["lane"]["unit"], "Decibels");
    assert_eq!(first["data"]["lane"]["pointCount"], 2);

    // 改模型：把整份工程换成"同一条泳道但只有单点" ⇒ 读数必须跟着变
    // （单点泳道处处保持 —— 这是模型 `value_at` 的边界口径）。
    let mut changed = filled_project();
    let target = AutomationTarget::TrackVolume { track_id: track };
    {
        let lane = changed
            .tracks
            .get_mut(&track)
            .and_then(|track| track.automation_lanes.get_mut(&target))
            .expect("样本里有这条泳道");
        lane.points.clear();
        let point_id = EntityId::new();
        lane.points.insert(
            point_id,
            AutomationPoint {
                id: point_id,
                tick: 0,
                value: -20.0,
                curve: CurveType::SCurve,
            },
        );
    }
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("changed"), changed, false)
        .expect("换一份工程");
    let second = call_tool(&mut dispatcher, "yeban_edit_automation", arguments);
    let values = second["data"]["read"]["values"].as_array().expect("values");
    assert_eq!(values[0]["automationValue"], -20.0);
    assert_eq!(
        values[1]["automationValue"], -20.0,
        "单点泳道处处保持（模型的边界口径）"
    );
    assert_eq!(values[2]["automationValue"], -20.0);
    assert_eq!(second["data"]["lane"]["pointCount"], 1);
}

// ---------------------------------------------------------------------------
// ① 写类：真的多了东西 + 逆操作逐字节回退
// ---------------------------------------------------------------------------

#[test]
fn automation_write_lands_in_the_project_and_undo_restores_byte_for_byte() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    let before = project_bytes(&dispatcher);
    let commits_before = dispatcher.domain().commit_count();

    let written = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "ticks": [1920],
            "point": {"tick": 1920, "value": -18.0, "curve": "SCurve"},
        }),
    );
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(written["data"]["applied"], true);
    assert_eq!(written["data"]["written"]["existed"], false);
    assert_eq!(written["data"]["written"]["op"], "SetAutomationPoint");
    assert_eq!(written["data"]["valuesAfter"][0]["automationValue"], -18.0);
    assert_eq!(
        written["data"]["valuesAfter"][0]["source"], "automation",
        "写完之后这一 tick 上真的有自动化值"
    );

    // 工程**真的**变了：字节不同、提交数 +1、模型里多了一个点。
    let after = project_bytes(&dispatcher);
    assert_ne!(after, before, "写类工具必须真的改工程");
    assert_eq!(dispatcher.domain().commit_count(), commits_before + 1);
    let target = AutomationTarget::TrackVolume { track_id: track };
    let lane_points = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .automation_lane(&target)
        .expect("泳道")
        .points
        .len();
    assert_eq!(lane_points, 3, "原有两个点 + 新写的一个");

    // 逆操作：`yeban_undo` 之后**逐字节**回到写之前的工程。
    let undone = call_tool(
        &mut dispatcher,
        "yeban_undo",
        serde_json::json!({"steps": 1}),
    );
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(project_bytes(&dispatcher), before, "撤销必须逐字节回退");
    // ⚠ 撤销**只移动游标, 不回退提交图谱**（`Plan::commit_delta` 对 Undo/Redo 恒为 0）:
    // 那次写入的提交仍在 DAG 里, 只是不再被应用。第一版这里写成"回到 commits_before"
    // 是错的 —— CI 用 `left: 2, right: 1` 把它抓出来了。
    assert_eq!(
        dispatcher.domain().commit_count(),
        commits_before + 1,
        "撤销不动提交图谱（只动游标）"
    );
    assert_eq!(
        undone["data"]["undoneTotal"], 1,
        "游标确实前进了一步: {undone}"
    );
    assert_eq!(
        undone["data"]["after"]["canRedo"], true,
        "撤销之后必须可以重做: {undone}"
    );

    // 重做再把同一个点写回来（同一实现、同一载荷）。
    let redone = call_tool(
        &mut dispatcher,
        "yeban_redo",
        serde_json::json!({"steps": 1}),
    );
    assert_eq!(redone["status"], "success", "{redone}");
    assert_eq!(project_bytes(&dispatcher), after, "重做必须逐字节复原");
}

// ---------------------------------------------------------------------------
// ① 来源标签：MCP **直接编辑**的作者是 `OpOrigin::McpEdit`（不是借来的变体）
// ---------------------------------------------------------------------------

/// **落盘日志**里两个直接编辑工具的作者标签必须是 `McpEdit`。
///
/// ⚠ 见证（`MUST-GATE-001` 的 I4 教训）：这条判据**先**证明日志里真的躺着这两次编辑的
/// `Op` 本体（`SetAutomationPoint` / `AddClip`），**再**断言它们的 `origin`。
/// 否则"在空集合上断言"会让判据在写路径整个断掉时**仍然是绿的**。
///
/// 观测面是容器里的 `history.dag`（`StampedOp` 的持久化形态），
/// 不是响应的 `origin.kind` —— 后者是给人看的标签，前者才是被审计的作者。
#[test]
fn direct_edits_are_authored_by_the_mcp_edit_origin_in_the_persisted_log() {
    let dir = std::env::temp_dir().join(format!(
        "yeban-mcp-ext-origin-log-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建目录");
    let path = dir.join("demo.yeban");

    let project = filled_project();
    let track = lead_track(&project);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path.clone(), project, false)
        .expect("注入规范工程");

    let audio = wav_s16(48_000, &[0, 1_000, -1_000, 0]);
    let audio_path = write_audio_fixture("origin-log", &audio);

    // 两次**直接编辑**（都不创建提案）：写一个自动化点 + 导入一份音频。
    let edit = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "point": {"tick": 1920, "value": -18.0, "curve": "SCurve"},
        }),
    );
    assert_eq!(edit["status"], "success", "{edit}");
    // 判据 ⑤（来源 → 审计显示）：响应里的来源块必须点名新变体，且不再出现借来的名字。
    assert_eq!(edit["data"]["origin"]["kind"], "McpEdit", "{edit}");
    assert_eq!(edit["data"]["origin"]["author"], "yeban-mcp", "{edit}");
    assert_ne!(edit["data"]["origin"]["kind"], "AutomationRecord");

    let imported = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({
            "name": "Kick",
            "path": audio_path.display().to_string(),
        }),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    assert_eq!(
        dispatcher.domain().commit_count(),
        3,
        "根提交 + 两次直接编辑"
    );

    // 落盘 —— 作者标签的权威形态在 `history.dag` 里。
    let saved = call_tool(
        &mut dispatcher,
        "yeban_save_project",
        serde_json::json!({"force": true}),
    );
    assert_eq!(saved["status"], "success", "{saved}");

    let rows = persisted_origins(&path);
    // 见证 1: 日志非空。
    assert!(!rows.is_empty(), "落盘日志不得为空 —— 否则下面的断言是空转");
    // 见证 2: 两次编辑的 **Op 本体**都在日志里。
    let automation = rows
        .iter()
        .find(|(_, op)| {
            matches!(op, Op::Batch { ops, .. }
                if ops.iter().any(|inner| matches!(inner, Op::SetAutomationPoint { .. })))
        })
        .unwrap_or_else(|| panic!("日志里必须有这次自动化编辑: {rows:?}"));
    assert!(
        matches!(automation.0, OpOrigin::McpEdit { .. }),
        "自动化直接编辑的作者必须是 McpEdit, 实际 {:?}",
        automation.0
    );
    let clip = rows
        .iter()
        .find(|(_, op)| {
            matches!(op, Op::Batch { ops, .. }
                if ops.iter().any(|inner| matches!(inner, Op::AddClip { .. })))
        })
        .unwrap_or_else(|| panic!("日志里必须有这次音频导入: {rows:?}"));
    assert!(
        matches!(clip.0, OpOrigin::McpEdit { .. }),
        "音频直接导入的作者必须是 McpEdit, 实际 {:?}",
        clip.0
    );

    // 作者名如实（与 `UndoState.author` 同源），且日志里不得再借那两个变体。
    for (origin, _) in &rows {
        if let OpOrigin::McpEdit { agent_name } = origin {
            assert_eq!(agent_name, "yeban-mcp");
        }
    }
    assert!(
        !rows
            .iter()
            .any(|(origin, _)| matches!(origin, OpOrigin::AutomationRecord | OpOrigin::Import)),
        "MCP 直接编辑不得再借 AutomationRecord/Import; 实际作者: {:?}",
        rows.iter()
            .map(|(origin, _)| origin.clone())
            .collect::<Vec<_>>()
    );

    std::fs::remove_dir_all(&dir).ok();
    if let Some(parent) = audio_path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

#[test]
fn rewriting_the_same_tick_updates_one_point_instead_of_stacking() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    let arguments = serde_json::json!({
        "trackId": track.to_canonical_string(),
        "lane": "TrackVolume",
        "point": {"tick": 960, "value": -1.0},
    });
    let first = call_tool(&mut dispatcher, "yeban_edit_automation", arguments.clone());
    let second = call_tool(&mut dispatcher, "yeban_edit_automation", arguments);
    assert_eq!(
        first["data"]["written"]["pointId"], second["data"]["written"]["pointId"],
        "同一 (目标, tick) ⇒ 同一点身份（确定性派生）"
    );
    assert_eq!(second["data"]["written"]["existed"], true);
    assert_eq!(second["data"]["written"]["oldValue"], -1.0);
    assert_eq!(
        second["data"]["lane"]["pointCount"], 3,
        "更新同一点 ⇒ 数量不增加"
    );
}

// ---------------------------------------------------------------------------
// ① 音频导入：真的登记 + 逆操作逐字节回退 + PcmBudget 真的生效
// ---------------------------------------------------------------------------

#[test]
fn import_audio_registers_a_clip_and_undo_restores_byte_for_byte() {
    let project = filled_project();
    let mut dispatcher = dispatcher_with_project(project);
    let before = project_bytes(&dispatcher);
    let pool_before = dispatcher.domain().asset_count();
    let bytes = wav_s16(48_000, &[0, 1_000, -1_000, 0]);
    let path = write_audio_fixture("import", &bytes);
    let expected_hash = AssetHash::of_bytes(&bytes);

    let imported = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({
            "name": "Kick",
            "path": path.display().to_string(),
            "gainDb": -1.5,
        }),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    assert_eq!(imported["data"]["created"], true);
    assert_eq!(imported["data"]["clip"]["asset"], expected_hash.as_str());
    assert_eq!(imported["data"]["decoded"]["sampleRate"], 48_000);
    assert_eq!(imported["data"]["decoded"]["frames"], 4);
    assert_eq!(imported["data"]["source"]["kind"], "diskPath");

    // 工程里**真的**多了一条音频片段池条目，且字节**真的**进了会话 CAS 池。
    let after = project_bytes(&dispatcher);
    assert_ne!(after, before);
    assert_eq!(dispatcher.domain().asset_count(), pool_before + 1);
    let clip_id = imported["data"]["clip"]["clipId"]
        .as_str()
        .expect("clipId")
        .to_owned();
    let clip = {
        use std::str::FromStr as _;
        let id = EntityId::from_str(&clip_id).expect("ULID");
        dispatcher
            .domain()
            .active_project()
            .expect("活跃工程")
            .clip_pool
            .get(&id)
            .cloned()
            .expect("片段池里必须有这一条")
    };
    match &clip.content {
        ClipContent::Audio { asset, gain_db } => {
            assert_eq!(asset, &expected_hash);
            assert_eq!(*gain_db, -1.5);
        }
        ClipContent::Midi { .. } => panic!("必须是音频片段"),
    }
    assert_eq!(
        dispatcher.domain().asset(&expected_hash),
        Some(bytes.as_slice()),
        "CAS 池里的字节必须逐字节等于磁盘上的容器字节"
    );

    // 逆操作逐字节回退工程（CAS 池的字节不是 Op 的载荷，见响应里的 notes）。
    let undone = call_tool(
        &mut dispatcher,
        "yeban_undo",
        serde_json::json!({"steps": 1}),
    );
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(
        project_bytes(&dispatcher),
        before,
        "撤销必须逐字节回退 clip_pool"
    );
    let notes = imported["data"]["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .is_some_and(|note| note.contains("撤销 clip_pool 条目不会回收池里的字节"))),
        "必须如实登记 CAS 池的边界: {notes:?}"
    );
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

#[test]
fn import_audio_accepts_a_pool_asset_and_respects_the_pcm_budget() {
    let project = filled_project();
    let mut dispatcher = dispatcher_with_project(project);
    // 先把字节放进会话 CAS 池（模拟"容器里已经有 assets/{sha256}"）。
    let bytes = wav_s16(44_100, &[0, 5, -5, 0]);
    let hash = dispatcher
        .domain_mut()
        .put_asset(bytes.clone())
        .expect("登记资产");
    assert_eq!(hash, AssetHash::of_bytes(&bytes));

    let imported = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "assetHash": hash.as_str()}),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    assert_eq!(imported["data"]["source"]["kind"], "assetPool");
    assert_eq!(
        imported["data"]["source"]["registeredBytes"], 0,
        "池里已有字节 ⇒ 不重复登记"
    );
    assert_eq!(imported["data"]["decoded"]["sampleRate"], 44_100);

    // 报出来的预算必须就是**唯一来源**（`yeban_decode::DecodeOptions::default()`），
    // 不是本 crate 抄的一份数字。
    let budget = &imported["data"]["budget"];
    assert_eq!(budget["gate"], "yeban_decode::DecodeOptions::default()");
    assert!(budget["maxPcmBytes"].as_u64().expect("maxPcmBytes") > 0);
    assert!(budget["maxDurationSecs"].as_u64().expect("时长上限") > 0);

    // 一个"头里声称 8 小时以上"的文件真的会撞上时长闸门 ⇒ 既有错误码 + 预算标记。
    // 3 GB 声明 @48kHz 单声道 16-bit = 1.5e9 帧 ÷ 48000 = 31250 s ≈ 8.7 小时 > 默认 6 小时上限。
    // ⚠ 声明长度**不能**取 `u32::MAX`：symphonia 把那个值当作"流式、长度未知"（`DataChunk::len`
    // 会变成 `None`），于是 `num_frames` 缺失、时长闸门根本不会触发 —— 实测口径见
    // `symphonia-format-riff` 的 `DataChunk::parse`。
    let huge_declared = wav_with_declared_len(48_000, &[0, 0, 0, 0], Some(3_000_000_000));
    let huge_path = write_audio_fixture("huge", &huge_declared);
    let refused = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({
            "name": "Huge",
            "path": huge_path.display().to_string(),
        }),
    );
    assert_eq!(refused["status"], "error", "{refused}");
    assert_eq!(refused["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(refused["error"]["data"]["budget"], true);
    assert_eq!(refused["error"]["data"]["reason"], "pcmBudgetExceeded");
    if let Some(parent) = huge_path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

// ---------------------------------------------------------------------------
// ① 设备与引擎：三份状态各有唯一来源
// ---------------------------------------------------------------------------

#[test]
fn engine_state_reads_three_single_sources() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    dispatcher
        .domain_mut()
        .set_engine_readings(Some(EngineReadings {
            sample_rate: 48_000,
            buffer_frames: 128,
            ..EngineReadings::default()
        }));
    dispatcher.domain_mut().session_mut().seek_ticks(3_840);
    dispatcher.domain_mut().session_mut().play();

    let reported = call_tool(
        &mut dispatcher,
        "yeban_query_engine_state",
        serde_json::json!({"trackId": track.to_canonical_string()}),
    );
    assert_eq!(reported["status"], "success", "{reported}");
    // 采样率：工程的 `audio_config.sample_rate`（唯一来源）。
    assert_eq!(reported["data"]["engine"]["sampleRate"], 48_000);
    assert_eq!(
        reported["data"]["engine"]["sampleRateSource"],
        "project.audio_config.sample_rate"
    );
    // 缓冲：宿主注入的镜像。
    assert_eq!(reported["data"]["engine"]["bufferFrames"], 128);
    assert_eq!(
        reported["data"]["engine"]["bufferSource"],
        "hostEngineMirror"
    );
    assert_eq!(reported["data"]["engine"]["sampleRateMatchesMirror"], true);
    // 走带：SessionRuntimeState（唯一来源）。
    assert_eq!(reported["data"]["session"]["playheadTicks"], 3_840);
    assert_eq!(reported["data"]["session"]["isPlaying"], true);
    assert_eq!(reported["data"]["session"]["transportState"], "playing");
    assert_eq!(reported["data"]["session"]["playheadBar"], 1);
    assert_eq!(reported["data"]["session"]["ticksPerBeat"], 960);
    // 设备链：工程文档（含 PDC 的延迟字段）。
    assert_eq!(reported["data"]["track"]["deviceCount"], 1);
    assert_eq!(
        reported["data"]["track"]["devices"][0]["latencySamples"],
        32
    );
    assert_eq!(reported["data"]["track"]["totalLatencySamples"], 32);
    assert_eq!(reported["data"]["track"]["devices"][0]["paramCount"], 2);
    assert!(reported["data"]["stateSources"]["bufferFrames"].is_string());

    // 改模型 ⇒ 读数跟着变：换一份采样率不同、设备链被拆掉的工程。
    let mut changed = filled_project();
    let changed_track = lead_track(&changed);
    changed.audio_config.sample_rate = SampleRate::Hz44100;
    if let Some(track) = changed.tracks.get_mut(&changed_track) {
        track.devices.clear();
    }
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("engine"), changed, false)
        .expect("换一份工程");
    let after = call_tool(
        &mut dispatcher,
        "yeban_query_engine_state",
        serde_json::json!({"trackId": changed_track.to_canonical_string()}),
    );
    assert_eq!(after["data"]["engine"]["sampleRate"], 44_100);
    assert_eq!(
        after["data"]["engine"]["sampleRateMatchesMirror"], false,
        "镜像与工程不一致必须看得见"
    );
    assert_eq!(after["data"]["track"]["deviceCount"], 0);
    assert_eq!(after["data"]["track"]["totalLatencySamples"], 0);
}

#[test]
fn engine_state_is_read_only_and_never_touches_the_session_or_the_mirror() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    let readings = EngineReadings {
        sample_rate: 48_000,
        buffer_frames: 256,
        ..EngineReadings::default()
    };
    dispatcher.domain_mut().set_engine_readings(Some(readings));
    dispatcher.domain_mut().session_mut().seek_ticks(960);
    let digest_before = dispatcher.domain().project_digest();
    let session_before: SessionRuntimeState = dispatcher.domain().session().clone();
    let commits_before = dispatcher.domain().commit_count();

    let _ = call_tool(
        &mut dispatcher,
        "yeban_query_engine_state",
        serde_json::json!({"trackId": track.to_canonical_string()}),
    );
    assert_eq!(dispatcher.domain().project_digest(), digest_before);
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
    assert_eq!(dispatcher.domain().session(), &session_before);
    assert_eq!(dispatcher.domain().engine_readings(), Some(readings));
}

// ---------------------------------------------------------------------------
// ② dryRun：状态一位不变 + 预览与真做一致
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_every_state_bit_identical() {
    let project = filled_project();
    let track = lead_track(&project);
    let bytes = wav_s16(48_000, &[0, 1, -1, 0]);
    let path = write_audio_fixture("dryrun", &bytes);
    let mut dispatcher = dispatcher_with_project(project);
    dispatcher
        .domain_mut()
        .set_engine_readings(Some(EngineReadings {
            sample_rate: 48_000,
            buffer_frames: 64,
            ..EngineReadings::default()
        }));
    dispatcher.domain_mut().session_mut().seek_ticks(1_920);
    let digest_before = dispatcher.domain().project_digest();
    let bytes_before = project_bytes(&dispatcher);
    let commits_before = dispatcher.domain().commit_count();
    let session_before = dispatcher.domain().session().clone();
    let assets_before = dispatcher.domain().asset_count();
    let undo_before = dispatcher.domain().undo_state().undone();

    for (name, arguments) in [
        (
            "yeban_edit_automation",
            serde_json::json!({
                "trackId": track.to_canonical_string(),
                "lane": "TrackVolume",
                "ticks": [0, 1920],
                "point": {"tick": 1920, "value": -18.0},
            }),
        ),
        (
            "yeban_query_engine_state",
            serde_json::json!({"trackId": track.to_canonical_string()}),
        ),
        (
            "yeban_import_audio",
            serde_json::json!({"name": "Kick", "path": path.display().to_string()}),
        ),
    ] {
        let mut arguments = arguments;
        arguments[tools::DRY_RUN_PARAM] = Value::from(true);
        let preview = call_tool(&mut dispatcher, name, arguments);
        assert_eq!(
            preview["status"], "success",
            "`{name}` 的 dryRun: {preview}"
        );
        assert_eq!(preview["data"][yeban_mcp::dispatch::DRY_RUN_FLAG], true);
        assert_eq!(preview["data"]["stateUnchanged"], true);
        assert_eq!(preview["data"]["tool"], name);
        assert_eq!(
            preview["data"]["specId"],
            tools::tool(name).expect("注册表").spec_id
        );

        // 状态**逐项**不变（工程字节 / 摘要 / 提交数 / 会话态 / CAS 池 / 撤销游标）。
        assert_eq!(
            project_bytes(&dispatcher),
            bytes_before,
            "`{name}` 改了工程"
        );
        assert_eq!(dispatcher.domain().project_digest(), digest_before);
        assert_eq!(dispatcher.domain().commit_count(), commits_before);
        assert_eq!(dispatcher.domain().session(), &session_before);
        assert_eq!(
            dispatcher.domain().asset_count(),
            assets_before,
            "`{name}` 改了 CAS 池"
        );
        assert_eq!(dispatcher.domain().undo_state().undone(), undo_before);
        assert_eq!(dispatcher.idempotency_len(), 0, "dryRun 不得写幂等缓存");
    }
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

/// `Plan::op()` 的稳定名字（预览信封里的 `plan`）。
fn expected_plan(tool: &str) -> &'static str {
    match tool {
        "yeban_edit_automation" => "edit_automation",
        "yeban_query_engine_state" => "engine_state",
        _ => "import_audio",
    }
}

/// 预览与真做必须逐字段相同的键（各自取自 `data()` / `describe()` 的公共部分）。
fn preview_keys(tool: &str) -> &'static [&'static str] {
    match tool {
        "yeban_edit_automation" => &[
            "trackId",
            "target",
            "targetLabel",
            "laneKind",
            "readOnly",
            "lane",
            "read",
            "written",
            "valuesAfter",
            "projectDigestBefore",
            "projectDigestAfter",
            "origin",
        ],
        "yeban_query_engine_state" => &["session", "engine", "track", "stateSources"],
        _ => &[
            "imported",
            "created",
            "unchanged",
            "clip",
            "placement",
            "source",
            "decoded",
            "budget",
            "declaredInIndex",
            "projectDigestBefore",
            "projectDigestAfter",
            "notes",
        ],
    }
}

#[test]
fn dry_run_preview_equals_the_real_call_for_all_three_tools() {
    let track = lead_track(&filled_project());
    let bytes = wav_s16(48_000, &[0, 1, -1, 0]);
    let path = write_audio_fixture("preview", &bytes);

    let cases: Vec<(&str, Value)> = vec![
        (
            "yeban_edit_automation",
            serde_json::json!({
                "trackId": track.to_canonical_string(),
                "lane": "TrackVolume",
                "ticks": [0, 1920, 3840],
                "point": {"tick": 1920, "value": -18.0, "curve": "SCurve"},
            }),
        ),
        (
            "yeban_query_engine_state",
            serde_json::json!({"trackId": track.to_canonical_string()}),
        ),
        (
            "yeban_import_audio",
            serde_json::json!({
                "name": "Kick",
                "path": path.display().to_string(),
                "gainDb": -2.0,
            }),
        ),
        // 同一个工具**带摆放**的那条路径也要走一遍预览对账：摆放的实参解析、
        // 时值缺省换算与确定性 `placementId` 都必须在 dryRun 与真做之间逐字段相同。
        (
            "yeban_import_audio",
            serde_json::json!({
                "name": "KickPlaced",
                "path": path.display().to_string(),
                "trackId": track.to_canonical_string(),
                "startTick": 960,
                "muted": true,
            }),
        ),
    ];
    for (name, arguments) in cases {
        // ⚠ **每个用例各自一对分发器**：第一版在循环外只建一对, 于是第一个用例的**真调用**
        // （写一个点）让两侧工程分叉, 后面两个用例的摘要必然不等 —— CI 用 `left/right`
        // 两个不同 digest 把它抓出来了。夹具的起点必须在**每个用例**上重新对齐。
        let mut preview_run = dispatcher_with_project(filled_project());
        let mut real_run = dispatcher_with_project(filled_project());
        let mut dry_arguments = arguments.clone();
        dry_arguments[tools::DRY_RUN_PARAM] = Value::from(true);
        let dry = call_tool(&mut preview_run, name, dry_arguments);
        let real = call_tool(&mut real_run, name, arguments);
        let preview = &dry["data"]["preview"];
        assert_eq!(preview["plan"], expected_plan(name), "预览必须自报计划名");

        // 逐字段对账：预览里的**每一件事实**都必须与真做相同
        // （两侧共用同一个 `data()` 函数，因此这是结构性成立；判据把它钉住）。
        for key in preview_keys(name) {
            assert!(
                preview.get(key).is_some(),
                "`{name}` 的预览缺少 `{key}`: {preview}"
            );
            assert_eq!(
                &preview[key], &real["data"][key],
                "`{name}` 的 dryRun 预览与真做在 `{key}` 上不一致"
            );
        }
        // 真做之后的**实测**摘要必须等于预览里的预测（最强的一条：它是运行期读数）。
        let actual_digest = Value::from(real_run.domain().project_digest().expect("工程摘要"));
        assert_eq!(
            actual_digest, preview["projectDigestAfter"],
            "`{name}` 真做之后的实测摘要必须等于预览里的预测"
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

// ---------------------------------------------------------------------------
// ③ 错误码：只用既有码（不发明新码）
// ---------------------------------------------------------------------------

/// 把一份 `ToolResponse` 的错误码取出并断言它落在 D25 的 20 值联集里。
fn assert_in_band_code(response: &Value, expected: ErrorCode) -> Value {
    assert_eq!(response["status"], "error", "{response}");
    let code = response["error"]["code"].as_str().expect("code");
    assert_eq!(code, expected.as_str(), "{response}");
    assert!(
        expected.is_schema_contract(),
        "{expected} 不在 ADR-0001 D25 的联集里"
    );
    response["error"]["data"].clone()
}

#[test]
fn bad_and_unknown_parameters_only_use_codes_inside_d25() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);

    // 未知 lane（别名）⇒ INVALID_PARAMETER_RANGE。
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({"trackId": track.to_canonical_string(), "lane": "trackVolume"}),
    );
    let data = assert_in_band_code(&response, ErrorCode::InvalidParameterRange);
    assert_eq!(data["field"], "lane");
    assert!(data["allowed"].is_array(), "必须告诉调用方合法取值: {data}");

    // SendGain 缺 edgeId ⇒ INVALID_PARAMETER_RANGE。
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({"trackId": track.to_canonical_string(), "lane": "SendGain"}),
    );
    let _ = assert_in_band_code(&response, ErrorCode::InvalidParameterRange);

    // 未知曲线名 ⇒ INVALID_PARAMETER_RANGE。
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "point": {"tick": 0, "value": 0.0, "curve": "sCurve"},
        }),
    );
    let _ = assert_in_band_code(&response, ErrorCode::InvalidParameterRange);

    // 值越出泳道取值域 ⇒ OUT_OF_RANGE。
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "point": {"tick": 0, "value": 99.0},
        }),
    );
    let _ = assert_in_band_code(&response, ErrorCode::OutOfRange);

    // 不存在的音轨 ⇒ TRACK_NOT_FOUND。
    let ghost = EntityId::new().to_canonical_string();
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({"trackId": ghost, "lane": "TrackVolume"}),
    );
    let _ = assert_in_band_code(&response, ErrorCode::TrackNotFound);

    // 设备插槽越界 ⇒ INDEX_OUT_OF_BOUNDS。
    let response = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "DeviceParam",
            "slotIndex": 9,
        }),
    );
    let _ = assert_in_band_code(&response, ErrorCode::IndexOutOfBounds);

    // 资产不在池里 ⇒ ENTITY_NOT_FOUND。
    let response = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "assetHash": AssetHash::of_bytes(b"nope").as_str()}),
    );
    let data = assert_in_band_code(&response, ErrorCode::EntityNotFound);
    assert_eq!(data["reason"], "assetNotInPool");

    // 两个来源都不给 / 都给 ⇒ INVALID_PARAMETER_RANGE（形状非法）。
    let response = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick"}),
    );
    assert_eq!(
        assert_in_band_code(&response, ErrorCode::InvalidParameterRange)["reason"],
        "sourceMissing"
    );
    let response = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "assetHash": "00".repeat(32), "path": "/tmp/x.wav"}),
    );
    assert_eq!(
        assert_in_band_code(&response, ErrorCode::InvalidParameterRange)["reason"],
        "sourceAmbiguous"
    );

    // 磁盘文件不存在 ⇒ FILE_NOT_FOUND；不是音频 ⇒ RENDER_FAILED（+ 分类）。
    let response = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "path": "/definitely/not/here/kick.wav"}),
    );
    let _ = assert_in_band_code(&response, ErrorCode::FileNotFound);

    let not_audio = write_audio_fixture("notaudio", b"definitely not audio");
    let response = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "path": not_audio.display().to_string()}),
    );
    let data = assert_in_band_code(&response, ErrorCode::RenderFailed);
    assert!(data["decodeError"].is_string(), "{data}");
    if let Some(parent) = not_audio.parent() {
        std::fs::remove_dir_all(parent).ok();
    }

    // 参数形状错（类型不对 / 拼错的键）走既有 JSON-RPC 码（-32602），**不**是新码。
    let (status, code) = call_tool_raw(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({"trackId": track.to_canonical_string(), "lane": 7}),
    );
    assert_eq!(status, 400);
    assert_eq!(code, Some(yeban_mcp::jsonrpc::INVALID_PARAMS));
    let (status, code) = call_tool_raw(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "dryrun": true,
        }),
    );
    assert_eq!(status, 400);
    assert_eq!(code, Some(yeban_mcp::jsonrpc::INVALID_PARAMS));
}

#[test]
fn error_code_vocabulary_is_the_contract_enum_exactly() {
    // 源码侧声明参与契约的集合 == 契约文件里的 enum（文本级；与契约判据同口径）。
    let sources = yeban_mcp::undo_session::read_rust_sources(
        &yeban_mcp::undo_session::production_source_roots(),
    );
    let schema =
        std::fs::read_to_string(repo_path("schemas/mcp-tools.schema.json")).expect("读契约");
    assert_eq!(
        extension_audit::scan_error_code_vocabulary(&sources, &schema),
        Vec::<String>::new()
    );
    // 运行时侧：每一个 `ErrorCode` 都在 20 值联集里（实现级 `NOT_IMPLEMENTED` 除外）。
    for code in ErrorCode::ALL {
        if code == ErrorCode::NotImplemented {
            assert!(!code.is_schema_contract());
            continue;
        }
        assert!(
            code.is_schema_contract(),
            "{code} 不在 ADR-0001 D25 的 20 值联集里"
        );
    }
}

// ---------------------------------------------------------------------------
// ④ 幂等键
// ---------------------------------------------------------------------------

#[test]
fn the_same_idempotency_key_never_applies_twice() {
    let project = filled_project();
    let track = lead_track(&project);
    let bytes = wav_s16(48_000, &[0, 1, -1, 0]);
    let path = write_audio_fixture("idem", &bytes);
    let mut dispatcher = dispatcher_with_project(project);

    for (name, arguments) in [
        (
            "yeban_edit_automation",
            serde_json::json!({
                "trackId": track.to_canonical_string(),
                "lane": "TrackVolume",
                "point": {"tick": 1920, "value": -9.0},
                "idempotencyKey": "k-automation",
            }),
        ),
        (
            "yeban_import_audio",
            serde_json::json!({
                "name": "Kick",
                "path": path.display().to_string(),
                "idempotencyKey": "k-import",
            }),
        ),
    ] {
        let first = call_tool(&mut dispatcher, name, arguments.clone());
        assert_eq!(first["status"], "success", "{first}");
        let digest_after_first = dispatcher.domain().project_digest();
        let commits_after_first = dispatcher.domain().commit_count();
        let assets_after_first = dispatcher.domain().asset_count();

        // 同键重放：命中缓存 ⇒ 主体逐字节相同 + 状态一位不动。
        let second = call_tool(&mut dispatcher, name, arguments.clone());
        assert_eq!(second["replayed"], true, "{second}");
        assert_eq!(
            &second["response"]["result"], &first,
            "重放主体必须与首次逐字节相同"
        );
        assert_eq!(dispatcher.domain().project_digest(), digest_after_first);
        assert_eq!(dispatcher.domain().commit_count(), commits_after_first);
        assert_eq!(dispatcher.domain().asset_count(), assets_after_first);
        assert!(dispatcher.replayed() > 0, "必须命中过幂等缓存");
    }
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

// ---------------------------------------------------------------------------
// ⑥ 不许出现第二份实现（审计在真实源码上真跑）
// ---------------------------------------------------------------------------

/// 仓库根（由 `CARGO_MANIFEST_DIR` 反推）。
fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

#[test]
fn no_second_automation_evaluation_in_production_sources() {
    let mcp = yeban_mcp::undo_session::read_rust_sources(&[repo_path("crates/yeban-mcp/src")]);
    let mut all = mcp.clone();
    all.extend(yeban_mcp::undo_session::read_rust_sources(&[repo_path(
        "crates/yeban-app/src",
    )]));
    assert!(all.len() >= 40, "应当扫到两个 crate 的生产源码");
    assert_eq!(
        automation_audit::scan_second_automation_evaluations(&all),
        Vec::<String>::new(),
        "生产代码里出现了第二份自动化求值（必须走 automation_value_at）"
    );
    // 反向：审计**确实**抓得住注入（否则这条判据是空转的）。
    let injected = vec![(
        "crates/yeban-mcp/src/domain/injected.rs".to_owned(),
        "pub fn second(lane: &Lane, tick: u64) -> Option<f32> { lane.value_at(tick) }\n".to_owned(),
    )];
    let found = automation_audit::scan_second_automation_evaluations(&injected);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("secondEvaluation"), "{found:?}");

    // 撤销侧的同类审计也必须在同一批源码上干净（两条审计共用同一批源码）。
    assert_eq!(
        yeban_mcp::undo_session::scan_second_undo_implementations(&all),
        Vec::<String>::new()
    );
}

/// 每个源文件都必须被 `mod` 声明（**CI 实测抓过一次**：`extension_audit.rs` 写好了
/// 却没进 `domain/mod.rs`，于是 `tests/extension_tools.rs` 的 import 直接 `E0432`）。
///
/// 为什么"本机能跑"这件事对这条特别重要：本机不编译这个含重依赖的 crate，
/// 而"文件在、模块不在"恰好是**只有编译才会发现**的错误。文本守卫把它提前到本机。
#[test]
fn every_source_file_is_declared_as_a_module() {
    let mcp = yeban_mcp::undo_session::read_rust_sources(&[repo_path("crates/yeban-mcp/src")]);
    assert_eq!(
        extension_audit::scan_orphan_modules(&mcp),
        Vec::<String>::new(),
        "有源文件没有被 `mod` 声明 —— 它不在 crate 里, 任何 import 都会 E0432"
    );
    // 反向：守卫**确实**抓得住孤儿（否则这条判据是空转的）。
    let injected = vec![
        (
            "crates/yeban-mcp/src/lib.rs".to_owned(),
            "pub mod domain;\n".to_owned(),
        ),
        (
            "crates/yeban-mcp/src/domain/mod.rs".to_owned(),
            "pub mod error;\n".to_owned(),
        ),
        (
            "crates/yeban-mcp/src/domain/injected_orphan.rs".to_owned(),
            "pub fn f() {}\n".to_owned(),
        ),
    ];
    let found = extension_audit::scan_orphan_modules(&injected);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("injected_orphan"), "{found:?}");
}

#[test]
fn the_write_paths_commit_through_the_single_undo_entry() {
    let mcp = yeban_mcp::undo_session::read_rust_sources(&[repo_path("crates/yeban-mcp/src")]);
    assert_eq!(
        extension_audit::scan_write_paths(&mcp),
        Vec::<String>::new(),
        "写类扩展工具必须经过 undo_session::commit（否则不可撤销）"
    );
}

#[test]
fn direct_edit_origins_in_production_sources_are_the_mcp_edit_variant() {
    let mcp = yeban_mcp::undo_session::read_rust_sources(&[repo_path("crates/yeban-mcp/src")]);
    assert_eq!(
        extension_audit::scan_direct_edit_origins(&mcp),
        Vec::<String>::new(),
        "直接编辑的来源标签必须是 OpOrigin::McpEdit（不得再借 AutomationRecord/Import）"
    );
    // 反向：守卫**确实**抓得住注入（否则这条判据是空转的）。
    let injected = vec![
        (
            "crates/yeban-mcp/src/domain/automation.rs".to_owned(),
            "fn apply() {\n    let request = CommitRequest {\n        \
             origin: OpOrigin::AutomationRecord,\n    };\n}\n"
                .to_owned(),
        ),
        (
            "crates/yeban-mcp/src/domain/import_audio.rs".to_owned(),
            "fn apply() {\n    let request = CommitRequest {\n        \
             origin: OpOrigin::Import,\n    };\n}\n"
                .to_owned(),
        ),
    ];
    let found = extension_audit::scan_direct_edit_origins(&injected);
    assert!(
        found
            .iter()
            .any(|line| line.contains("OpOrigin::AutomationRecord")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|line| line.contains("OpOrigin::Import")),
        "{found:?}"
    );
}

#[test]
fn dry_run_entry_points_take_shared_references_only() {
    let mcp = yeban_mcp::undo_session::read_rust_sources(&[repo_path("crates/yeban-mcp/src")]);
    assert_eq!(
        extension_audit::scan_dry_run_entry_points(&mcp),
        Vec::<String>::new(),
        "计划入口必须只拿共享引用（dryRun 的结构性保证）"
    );
}

#[test]
fn production_region_agrees_across_the_three_copies() {
    // 三份 `production_region`（undo_session / automation_audit / extension_audit）必须同口径；
    // 后两份为了让"裸 rustc 独立跑"而不能依赖 crate，所以这里把三份拉齐比对。
    let sources = yeban_mcp::undo_session::read_rust_sources(
        &yeban_mcp::undo_session::production_source_roots(),
    );
    assert!(!sources.is_empty());
    for (path, text) in &sources {
        assert_eq!(
            yeban_mcp::undo_session::production_region(text),
            automation_audit::production_region(text),
            "{path}: undo_session 与 automation_audit 的生产区口径不一致"
        );
        assert_eq!(
            yeban_mcp::undo_session::production_region(text),
            extension_audit::production_region(text),
            "{path}: undo_session 与 extension_audit 的生产区口径不一致"
        );
    }
}

// ---------------------------------------------------------------------------
// ⑦ / 会话态不变量：与既有十工具互不干扰
// ---------------------------------------------------------------------------

#[test]
fn the_session_mirror_never_drifts_from_the_undo_authority() {
    let project = filled_project();
    let track = lead_track(&project);
    let mut dispatcher = dispatcher_with_project(project);
    assert_eq!(
        dispatcher.domain().session().undo_cursor.undone(),
        dispatcher.domain().undo_state().undone()
    );

    // 走一遍会动游标与不动游标的混合序列。
    let _ = call_tool(
        &mut dispatcher,
        "yeban_edit_automation",
        serde_json::json!({
            "trackId": track.to_canonical_string(),
            "lane": "TrackVolume",
            "point": {"tick": 1920, "value": -9.0},
        }),
    );
    let _ = call_tool(
        &mut dispatcher,
        "yeban_query_engine_state",
        serde_json::json!({}),
    );
    let _ = call_tool(
        &mut dispatcher,
        "yeban_undo",
        serde_json::json!({"steps": 1}),
    );
    let _ = call_tool(
        &mut dispatcher,
        "yeban_redo",
        serde_json::json!({"steps": 1}),
    );
    let _ = call_tool(
        &mut dispatcher,
        "yeban_undo",
        serde_json::json!({"steps": 1}),
    );
    let _ = call_tool(
        &mut dispatcher,
        "yeban_import_audio",
        serde_json::json!({"name": "Kick", "assetHash": AssetHash::of_bytes(b"x").as_str()}),
    );
    assert_eq!(
        dispatcher.domain().session().undo_cursor.undone(),
        dispatcher.domain().undo_state().undone(),
        "模型会话态里的撤销游标镜像必须与权威游标一致"
    );
}

#[test]
fn the_documented_ten_tools_are_untouched() {
    // D46 的特别授权只允许**新增**工具定义：规范十工具的名称、顺序、specId、参数集、
    // 错误码必须与"十工具口径"完全一致（契约侧的对账在 tests/contract.rs）。
    assert_eq!(tools::TOOLS.len(), tools::TOOL_COUNT);
    assert_eq!(
        tools::TOOL_COUNT,
        tools::DOCUMENTED_TOOL_COUNT + tools::EXTENSION_TOOL_COUNT
    );
    for (index, spec) in tools::TOOLS
        .iter()
        .take(tools::DOCUMENTED_TOOL_COUNT)
        .enumerate()
    {
        assert_eq!(
            spec.spec_id,
            format!("{}{:03}", tools::SPEC_ID_PREFIX, index + 1),
            "规范十工具的顺序与编号不得改动"
        );
        // 作用域口径与 `tools.rs` 自己的判据一致：只有 save/close 是 app:save。
        if matches!(spec.name, "yeban_save_project" | "yeban_close_project") {
            assert_eq!(spec.scope, Scope::AppSave, "{} 的作用域被改动", spec.name);
        } else {
            assert_eq!(spec.scope, Scope::AppAdmin, "{} 的作用域被改动", spec.name);
        }
    }
    // 扩展段的规范 ID 一律带 EXT 前缀且**不**伪装成编号（D46 的"不许发明规范编号"）。
    for spec in tools::TOOLS.iter().skip(tools::DOCUMENTED_TOOL_COUNT) {
        assert!(spec.spec_id.starts_with(tools::EXTENSION_SPEC_ID_PREFIX));
        assert!(
            !spec
                .spec_id
                .trim_start_matches(tools::EXTENSION_SPEC_ID_PREFIX)
                .chars()
                .all(|character| character.is_ascii_digit())
        );
    }
    // 工具名集合唯一（既有十工具 + 扩展 = TOOL_COUNT，且不重复）。
    let names: BTreeSet<&str> = tools::tool_names().into_iter().collect();
    assert_eq!(names.len(), tools::TOOL_COUNT);
}

// ---------------------------------------------------------------------------
// ⑨ `yeban_export_midi`：只读 SMF 导出 + **内存**往返（全程不碰文件系统）
// ---------------------------------------------------------------------------

/// 测试侧的**独立** base64 解码器（RFC 4648 §4）—— 与被测的编码器分开写。
///
/// 存在意义同 `yeban-midi` 的自研 VLQ 回读：只有"另一份实现"才能证明字节层面的往返，
/// 用被测编码器自己解自己等于自证。
fn decode_base64(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("非法的 base64 字符: {other}"),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    out
}

/// 内存往返：`yeban_export_midi` 回传的 base64 解回来必须**逐字节等于**共享映射
/// （`yeban_midi::export::export_from_project` + `to_smf_bytes`）直接产出的字节，
/// 且这些字节被 `yeban_midi::midi::parse_smf` 读回后的**音符数**与导出侧一致。
///
/// 本判据**不碰文件系统**：工程由 `open_in_memory` 注入，字节只经 JSON 回传。
#[test]
fn export_midi_round_trips_in_memory_through_the_shared_mapping() {
    // 独立事实源 ①：共享映射层自己报的音符数（不是从工具响应里抄的）。
    let project = filled_project();
    let mapping = yeban_midi::export::export_from_project(&project).expect("filled 样本必须可映射");
    let expected_notes: usize = mapping.tracks.iter().map(|track| track.notes.len()).sum();
    assert_eq!(
        expected_notes, 4,
        "filled 样本的 MIDI 事实（`yeban-midi` 的判据 ⑤：一条轨道 / 四颗音符）"
    );

    let mut dispatcher = dispatcher_with_project(project);
    let digest_before = dispatcher.domain().project_digest();
    let commits_before = dispatcher.domain().commit_count();

    let response = call_tool(&mut dispatcher, "yeban_export_midi", serde_json::json!({}));
    assert_eq!(response["status"], "success", "{response}");
    let data = &response["data"];
    assert_eq!(data["format"], "smf1", "恒为 SMF 1");
    assert_eq!(data["formatNumber"], 1);
    assert_eq!(
        data["ppq"], 960,
        "时间分度 = 工程的 960 PPQ [MODEL-AST-001]"
    );
    assert_eq!(data["tracks"], mapping.tracks.len());
    assert_eq!(data["notes"], expected_notes);
    assert_eq!(data["tempos"], mapping.tempos.len());
    assert_eq!(data["encoding"], "base64");
    assert_eq!(data["source"], "yeban_midi::export::export_from_project");
    assert_eq!(data["readOnly"], true);

    // 独立事实源 ②：把回传的 base64 解回来 —— 必须**逐字节等于**共享映射编出来的字节。
    let bytes = decode_base64(data["content"].as_str().expect("content"));
    assert_eq!(
        bytes,
        mapping.to_smf_bytes().expect("共享编码器"),
        "工具回传的字节必须与共享映射逐字节相同（不许有第二份实现）"
    );
    assert_eq!(
        u64::try_from(bytes.len()).expect("小尺寸"),
        data["bytes"].as_u64().expect("bytes"),
        "报出的字节数必须是实测长度"
    );
    assert_eq!(data["sha256"], AssetHash::of_bytes(&bytes).as_str());

    // 独立事实源 ③：SMF 读取面把字节读回来 —— 音符数必须与导出侧一致。
    let parsed = yeban_midi::midi::parse_smf(&bytes).expect("导出的字节必须能被 SMF 读取面读回");
    assert_eq!(
        parsed.notes.len(),
        expected_notes,
        "音符数必须与共享映射一致"
    );
    assert_eq!(
        parsed.notes.len(),
        usize::try_from(data["notes"].as_u64().expect("notes")).expect("小尺寸")
    );
    assert_eq!(
        parsed.tempos.len(),
        usize::try_from(data["tempos"].as_u64().expect("tempos")).expect("小尺寸")
    );
    assert_eq!(parsed.ppq, 960);
    assert_eq!(parsed.format, mapping.format, "SMF 1");
    for note in &parsed.notes {
        assert_eq!(note.channel, 0, "filled 样本只有一条 MIDI 轨道 ⇒ 通道 0");
    }

    // 只读：工程状态一位都没改（本判据全程没有落盘）。
    assert_eq!(dispatcher.domain().project_digest(), digest_before);
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
}

/// ② 号口径对新工具同样成立：`dryRun` 预览与真做**逐字段一致**，且状态一位不变。
#[test]
fn export_midi_dry_run_preview_equals_the_real_call() {
    let mut preview_run = dispatcher_with_project(filled_project());
    let mut real_run = dispatcher_with_project(filled_project());
    let dry = call_tool(
        &mut preview_run,
        "yeban_export_midi",
        serde_json::json!({"dryRun": true}),
    );
    let real = call_tool(&mut real_run, "yeban_export_midi", serde_json::json!({}));
    let preview = &dry["data"]["preview"];
    assert_eq!(preview["plan"], "export_midi", "预览必须自报计划名");
    for key in [
        "format",
        "formatNumber",
        "ppq",
        "tracks",
        "notes",
        "tempos",
        "bytes",
        "encoding",
        "content",
        "sha256",
        "source",
        "encoder",
        "readOnly",
    ] {
        assert!(preview.get(key).is_some(), "预览缺少 `{key}`: {preview}");
        assert_eq!(
            &preview[key], &real["data"][key],
            "`yeban_export_midi` 的 dryRun 预览与真做在 `{key}` 上不一致"
        );
    }
    assert_eq!(dry["data"]["wouldChangeState"], false, "只读工具");
    assert_eq!(dry["data"]["stateUnchanged"], true);
    assert_eq!(
        preview_run.domain().project_digest(),
        real_run.domain().project_digest(),
        "dryRun 之后两侧工程仍然相同"
    );
}

// ---------------------------------------------------------------------------
// ⑨ [D56] 诊断包：响应 flag 与包内字节的一致性（隐私默认的牙）
// ---------------------------------------------------------------------------

/// 诊断包的输出目录（保证此刻不存在；测试自己建、自己删）。
fn diagnostics_out_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yeban-mcp-diag-{}-{tag}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ))
}

/// `[D56]` **包内字节与响应 `projectIncluded` 不许互相矛盾**。
///
/// 判据的牙长在"**iff**"上，而不是"包里有/没有某个文件"：
///
/// 1. 标记集合**从已知工程派生**（标题 / 音轨名 / 音轨 id / 片段 id / 资产哈希），
///    不是硬编码字面量 —— 换一份样本，标记跟着换；
/// 2. **阳性对照**：每个标记都断言**真的**出现在工程文档里。少了这一步，
///    "标记没命中"既可能是"包干净"，也可能是"标记根本不存在"，判据就是空的；
/// 3. 打开**真实 zip**（`yeban_model::container::read_container`，与读 `.yeban` 容器同一
///    读取器 ⇒ 零新增依赖），逐条目扫描：命中任一标记 ⟺ `projectIncluded == true`。
///
/// 方向是双向的：包内偷偷塞进工程文档 ⇒ 红（flag 说 false，字节说 true）；
/// flag 被改成 true 而包里没有工程 ⇒ 也红。注入实验（换回
/// `serde_json::to_string(project)` 喂 `state_json`）实测输出见提交说明。
#[test]
fn diagnostics_bundle_content_matches_the_project_included_flag() {
    let project = filled_project();
    // 标记从工程派生：标题 + 音轨名 + 音轨 id + 片段 id + 资产哈希。
    let mut markers: Vec<String> = vec![project.title.clone()];
    markers.extend(project.tracks.values().map(|track| track.name.clone()));
    markers.extend(project.tracks.keys().map(EntityId::to_canonical_string));
    markers.extend(project.clip_pool.keys().map(EntityId::to_canonical_string));
    markers.extend(project.assets.keys().map(|hash| hash.as_str().to_owned()));
    // 去重 + 防空：空标记集合会让"未命中"恒真 —— 那就是没有牙。
    let markers: Vec<String> = markers
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert!(markers.len() >= 8, "标记集合太小, 判据无牙: {markers:?}");

    // 阳性对照：每个标记**必须真的**在工程文档里出现（否则它是假标记）。
    let project_json = serde_json::to_string(&project).expect("序列化工程");
    for marker in &markers {
        assert!(
            project_json.contains(marker.as_str()),
            "标记 {marker:?} 不在工程文档里 —— 它不能用来判定包是否含工程"
        );
    }

    let out_dir = diagnostics_out_dir("privacy");
    std::fs::create_dir_all(&out_dir).expect("建诊断包输出目录");
    let mut dispatcher = dispatcher_with_project(project);
    let response = call_tool(
        &mut dispatcher,
        "yeban_export_diagnostics",
        serde_json::json!({ "outDir": out_dir.display().to_string() }),
    );
    assert_eq!(response["status"], "success", "{response}");
    let flag = response["data"]["projectIncluded"]
        .as_bool()
        .expect("`projectIncluded` 必须是布尔 —— 它是这个包唯一的隐私读数");

    // 打开**真实产物**（不是计划里的中间值）。
    let zip = std::fs::read(
        response["data"]["path"]
            .as_str()
            .expect("响应必须给出完整包路径"),
    )
    .expect("读回诊断包");
    let archive = yeban_model::container::read_container(
        &zip,
        &yeban_model::container::ContainerLimits::default(),
    )
    .expect("诊断包必须是可解析的 zip");

    // 去掉工程内容之后包**依然有用**：四类必需条目一个都不能少。
    for required in [
        "MANIFEST.txt",
        "env.txt",
        "git.txt",
        "engine-state.json",
        "config.json",
    ] {
        assert!(
            archive.get(required).is_some(),
            "诊断包缺必需条目 {required}: {:?}",
            archive.names().collect::<Vec<_>>()
        );
    }

    let hits: Vec<(String, String)> = archive
        .entries()
        .iter()
        .flat_map(|entry| {
            let text = String::from_utf8_lossy(&entry.data).into_owned();
            markers
                .iter()
                .filter(|marker| text.contains(marker.as_str()))
                .map(|marker| (entry.name.clone(), marker.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(
        hits.is_empty(),
        !flag,
        "包内字节与 `projectIncluded` 矛盾: flag={flag}, 命中={hits:?} —— \
         要么把工程文档从包里彻底拿掉 (改 `domain/diagnostics.rs` 里喂 `state_json` 的东西), \
         要么把 flag 改成 true 并同步模块文档/注册表"
    );
    std::fs::remove_dir_all(&out_dir).ok();
}
