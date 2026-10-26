//! `yeban_import_midi` (`[MCP-TOOL-EXT-IMPORT-MIDI]`, `[ARCH-EXT-002]`) 扩展工具集成测试。
//!
//! 覆盖：
//! 1. 工具在注册表、EXTENSION_NAMES、schema enum 与 $defs 中的登记与元数据；
//! 2. dryRun 预览 vs 实际执行，以及通过 `yeban_undo` 撤销后逐字节还原工程；
//! 3. 错误处理覆盖：CONFLICT（目标片段非空）、FILE_NOT_FOUND（文件不存在）、
//!    INVALID_PARAMETER_RANGE（空路径、损坏的 MIDI 字节）、NO_ACTIVE_PROJECT（无活跃工程）。

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};
use yeban_mcp::tools::{self, ErrorCode, SideEffect, TOOL_COUNT};
use yeban_mcp::undo_session::canonical_project_bytes;
use yeban_model::samples::{filled_project, project_with_notes};
use yeban_model::{EntityId, YebanProjectV1};

fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-import-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn dispatcher_with_project(project: YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("fixture"), project, false)
        .expect("注入工程");
    dispatcher
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
    let line = json!({
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
    let response = outcome.response.expect("响应");
    response.result.expect("result")
}

fn project_bytes(dispatcher: &Dispatcher) -> Vec<u8> {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    canonical_project_bytes(project).expect("规范化字节")
}

/// 构造一份真实的 SMF 文件并返回其文件路径。
fn create_test_smf_file() -> PathBuf {
    let export = yeban_midi::export::export_from_project(&filled_project()).expect("导出映射");
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-import-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写测试 SMF 文件");
    path
}

#[test]
fn test_import_midi_tool_registration() {
    let spec = tools::tool("yeban_import_midi").expect("yeban_import_midi 必须在 TOOLS 中注册");
    assert_eq!(spec.spec_id, "MCP-TOOL-EXT-IMPORT-MIDI");
    assert_eq!(spec.side_effect, SideEffect::ProjectState);
    assert_eq!(spec.scope, Scope::AppAdmin);
    assert!(tools::EXTENSION_NAMES.contains(&"yeban_import_midi"));

    let schema: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("schemas")
                .join("mcp-tools.schema.json"),
        )
        .expect("读取 schema"),
    )
    .expect("解析 schema");

    let enum_names: Vec<&str> = schema["definitions"]["ToolCall"]["properties"]["name"]["enum"]
        .as_array()
        .expect("enum 数组")
        .iter()
        .map(|v| v.as_str().expect("str"))
        .collect();
    assert!(enum_names.contains(&"yeban_import_midi"));
    assert_eq!(enum_names.len(), TOOL_COUNT);

    let defs = &schema["definitions"]["ExtensionToolArguments"]["$defs"]["yeban_import_midi"];
    assert!(defs.is_object());
    assert_eq!(defs["required"], json!(["path"]));
    assert!(defs["properties"]["path"].is_object());
    assert!(defs["properties"]["trackId"].is_object());
    assert!(defs["properties"]["clipId"].is_object());
}

#[test]
fn test_import_midi_dry_run_preview_vs_real_execution() {
    let smf_path = create_test_smf_file();
    let project = project_with_notes(0);
    let mut dispatcher = dispatcher_with_project(project);

    let before_bytes = project_bytes(&dispatcher);
    let before_commits = dispatcher.domain().commit_count();

    // 1. dryRun: true
    let dry_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
            "dryRun": true,
        }),
    );
    assert_eq!(dry_res["status"], "success", "{dry_res}");
    let dry_data = &dry_res["data"];
    assert_eq!(dry_data["dryRun"], true);
    assert_eq!(dry_data["preview"]["plan"], "import_midi");
    assert_eq!(dry_data["preview"]["wouldApply"], true);
    assert_eq!(dry_data["wouldChangeState"], true);
    assert_eq!(dry_data["stateUnchanged"], true);
    assert_eq!(dispatcher.domain().commit_count(), before_commits);
    assert_eq!(project_bytes(&dispatcher), before_bytes);

    // 2. 真实执行
    let real_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
        }),
    );
    assert_eq!(real_res["status"], "success", "{real_res}");
    let real_data = &real_res["data"];
    assert_eq!(real_data["applied"], true);
    assert!(real_data["noteCount"].as_u64().expect("noteCount") > 0);
    assert_eq!(
        real_data["commitCount"],
        dry_data["preview"]["commitCountAfter"]
    );
    assert_eq!(dispatcher.domain().commit_count(), before_commits + 1);
    assert_ne!(
        project_bytes(&dispatcher),
        before_bytes,
        "导入后工程字节必须发生改变"
    );

    // 3. 撤销回退：byte-for-byte 完全还原
    let undo_res = call_tool(&mut dispatcher, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo_res["status"], "success", "{undo_res}");
    assert_eq!(project_bytes(&dispatcher), before_bytes);

    let _ = fs::remove_file(smf_path);
}

#[test]
fn test_import_midi_fault_handling() {
    let smf_path = create_test_smf_file();

    // 1. CONFLICT: 目标 clip 非空（filled_project 内已有音符）
    {
        let mut dispatcher = dispatcher_with_project(filled_project());
        let res = call_tool(
            &mut dispatcher,
            "yeban_import_midi",
            json!({
                "path": smf_path.to_str().expect("str"),
            }),
        );
        assert_eq!(res["status"], "error", "{res}");
        assert_eq!(res["error"]["code"], ErrorCode::Conflict.as_str());
    }

    // 2. FILE_NOT_FOUND: 文件不存在
    {
        let mut dispatcher = dispatcher_with_project(project_with_notes(0));
        let non_existent = std::env::temp_dir().join("does-not-exist-yeban-smf.mid");
        let res = call_tool(
            &mut dispatcher,
            "yeban_import_midi",
            json!({
                "path": non_existent.to_str().expect("str"),
            }),
        );
        assert_eq!(res["status"], "error", "{res}");
        assert_eq!(res["error"]["code"], ErrorCode::FileNotFound.as_str());
    }

    // 3. INVALID_PARAMETER_RANGE: 空路径
    {
        let mut dispatcher = dispatcher_with_project(project_with_notes(0));
        let res = call_tool(
            &mut dispatcher,
            "yeban_import_midi",
            json!({
                "path": "",
            }),
        );
        assert_eq!(res["status"], "error", "{res}");
        assert_eq!(
            res["error"]["code"],
            ErrorCode::InvalidParameterRange.as_str()
        );
    }

    // 4. INVALID_PARAMETER_RANGE: 损坏 / 非法 MIDI 文件
    {
        let corrupt_path = std::env::temp_dir().join(format!(
            "yeban-corrupt-{}.mid",
            EntityId::new().to_canonical_string()
        ));
        fs::write(&corrupt_path, b"NOT_A_VALID_MIDI_FILE").expect("写损坏文件");

        let mut dispatcher = dispatcher_with_project(project_with_notes(0));
        let res = call_tool(
            &mut dispatcher,
            "yeban_import_midi",
            json!({
                "path": corrupt_path.to_str().expect("str"),
            }),
        );
        assert_eq!(res["status"], "error", "{res}");
        assert_eq!(
            res["error"]["code"],
            ErrorCode::InvalidParameterRange.as_str()
        );

        let _ = fs::remove_file(corrupt_path);
    }

    // 5. NO_ACTIVE_PROJECT: 无活跃工程
    {
        let token = BearerToken::generate().token;
        let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
        let res = call_tool(
            &mut dispatcher,
            "yeban_import_midi",
            json!({
                "path": smf_path.to_str().expect("str"),
            }),
        );
        assert_eq!(res["status"], "error", "{res}");
        assert_eq!(res["error"]["code"], ErrorCode::NoActiveProject.as_str());
    }

    let _ = fs::remove_file(smf_path);
}
