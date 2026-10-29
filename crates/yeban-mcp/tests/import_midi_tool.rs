//! `yeban_import_midi` (`[MCP-TOOL-EXT-IMPORT-MIDI]`, `[ARCH-EXT-002]`) 扩展工具集成测试。
//!
//! 覆盖：
//! 1. 工具在注册表、EXTENSION_NAMES、schema enum 与 $defs 中的登记与元数据；
//! 2. dryRun 预览 vs 实际执行，以及通过 `yeban_undo` 撤销后逐字节还原工程；
//! 3. 错误处理覆盖：CONFLICT（目标片段非空）、FILE_NOT_FOUND（文件不存在）、
//!    INVALID_PARAMETER_RANGE（空路径、损坏的 MIDI 字节）、NO_ACTIVE_PROJECT（无活跃工程）；
//! 4. ⭐ [ARCH-EXT-002]：导体轨标题 / 版权声明在**空工程**上的采纳上报
//!    （`adoptedTitle` / `adoptedCopyright`、dryRun 不落盘、undo 还原）；
//! 5. ⭐ [ARCH-EXT-002]：tick > 0 变速 / 拍号映射点的采纳上报
//!    （`adoptedTempoMap` / `adoptedTimeSignatureMap`、dryRun 不落盘、undo 还原）。

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
    assert!(defs["properties"]["splitTracks"].is_object());
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

fn create_multitrack_smf_file() -> PathBuf {
    use yeban_midi::midi::{DEFAULT_PPQ, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
    let export = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2)],
        tracks: vec![
            MidiExportTrack {
                name: "Track One".to_string(),
                channel: 0,
                notes: vec![yeban_model::MidiNote::new(EntityId::new(), 0, 60, 480)],
                control_changes: Vec::new(),
            },
            MidiExportTrack {
                name: "Track Two".to_string(),
                channel: 1,
                notes: vec![yeban_model::MidiNote::new(EntityId::new(), 480, 48, 480)],
                control_changes: Vec::new(),
            },
        ],
        key_signature: None,
        title: None,
        copyright: None,
    };
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-multitrack-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写测试多轨 SMF 文件");
    path
}

#[test]
fn test_import_midi_multitrack_split_vs_no_split() {
    let smf_path = create_multitrack_smf_file();
    let project = project_with_notes(0);
    let mut dispatcher = dispatcher_with_project(project);

    let before_bytes = project_bytes(&dispatcher);
    let before_tracks = dispatcher.domain().active_project().unwrap().tracks.len();

    // 1. 默认 splitTracks: true 导入
    let res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
        }),
    );
    assert_eq!(res["status"], "success", "{res}");
    let data = &res["data"];
    assert_eq!(data["noteCount"], 2);
    assert_eq!(data["losses"]["track_partitions_dropped"], 0);

    let created_tracks = data["createdTracks"]
        .as_array()
        .unwrap_or_else(|| panic!("拆分导入必须报出 createdTracks 数组: {data}"));
    assert_eq!(
        created_tracks.len(),
        1,
        "1 条目标轨 + 1 条拆分新建轨 ⇒ createdTracks 恰 1 项: {data}"
    );
    assert!(
        created_tracks[0]["trackId"].is_string(),
        "createdTracks[0].trackId 必须是字符串: {data}"
    );
    assert!(
        created_tracks[0]["clipId"].is_string(),
        "createdTracks[0].clipId 必须是字符串: {data}"
    );
    assert!(
        created_tracks[0]["name"].is_string(),
        "createdTracks[0].name 必须是字符串: {data}"
    );
    assert_eq!(created_tracks[0]["name"], "Track Two", "{data}");

    let after_project = dispatcher.domain().active_project().unwrap();
    assert_eq!(
        after_project.tracks.len(),
        before_tracks + 1,
        "多轨拆分必须增加一条新音轨"
    );

    let new_track = after_project
        .tracks
        .values()
        .find(|t| t.name == "Track Two")
        .expect("必须找到拆分出的 Track Two 音轨");
    assert_eq!(new_track.kind, yeban_model::TrackKind::Midi);
    assert_eq!(new_track.clips.len(), 1);

    // 验证路由连接到 master
    let edge_exists = after_project.routing_graph.edges.values().any(|e| {
        e.source_node == new_track.id && e.destination_node == after_project.master_bus_track_id
    });
    assert!(edge_exists, "新音轨必须有连接到 master 总线的路由边");

    // 2. 撤销
    let undo_res = call_tool(&mut dispatcher, "yeban_undo", json!({ "steps": 1 }));
    assert_eq!(undo_res["status"], "success", "{undo_res}");
    assert_eq!(project_bytes(&dispatcher), before_bytes);

    // 3. 显式 splitTracks: false 导入
    let res_no_split = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
            "splitTracks": false,
        }),
    );
    assert_eq!(res_no_split["status"], "success", "{res_no_split}");
    let data_no_split = &res_no_split["data"];
    assert_eq!(data_no_split["noteCount"], 2);
    assert_eq!(data_no_split["losses"]["track_partitions_dropped"], 2);
    assert_eq!(
        data_no_split["createdTracks"],
        json!([]),
        "splitTracks: false 时 createdTracks 必须为空数组"
    );

    let no_split_project = dispatcher.domain().active_project().unwrap();
    assert_eq!(
        no_split_project.tracks.len(),
        before_tracks,
        "splitTracks: false 时音轨总数不得增加"
    );

    let _ = fs::remove_file(smf_path);
}

/// 造一份**带唯一 tick 0 调号**（`FF 59`：2 升、大调 = D Major）的 SMF（`Parallel`：
/// track 0 是导体轨，调号写在导体轨首元事件）。
fn create_key_signature_smf_file() -> PathBuf {
    use yeban_midi::midi::{DEFAULT_PPQ, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
    let export = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2)],
        tracks: vec![MidiExportTrack {
            name: "Piano".to_string(),
            channel: 0,
            notes: vec![yeban_model::MidiNote::new(EntityId::new(), 0, 60, 480)],
            control_changes: Vec::new(),
        }],
        key_signature: Some(yeban_model::project::KeySignature {
            sharps_flats: 2,
            is_minor: false,
        }),
        title: None,
        copyright: None,
    };
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-import-keysig-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写带调号的 SMF 文件");
    path
}

/// 一个**没有内容**且 `key_signature` 为空的工程（调号采纳门槛：目标工程无内容，
/// ⛔ 不覆盖已有值，非空工程一位都不动）。
fn project_without_key_signature() -> YebanProjectV1 {
    let mut project = yeban_midi::import::content_free_project();
    project.key_signature = None;
    project
}

/// 判据（[MCP-TOOL-EXT-IMPORT-MIDI] / [ARCH-EXT-002]）：唯一 tick 0 调号在**空工程**上
/// 被采纳进 `project.key_signature`，响应逐条回报 `adoptedKeySignature`；dryRun 不落盘；
/// 一次 undo 完全还原。
#[test]
fn test_import_midi_adopts_key_signature() {
    let smf_path = create_key_signature_smf_file();
    let mut dispatcher = dispatcher_with_project(project_without_key_signature());

    let before_bytes = project_bytes(&dispatcher);
    let before_commits = dispatcher.domain().commit_count();

    // 1. dryRun：预览如实报出将要采纳的调号，工程一位都不动。
    let dry_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
            "dryRun": true,
        }),
    );
    assert_eq!(dry_res["status"], "success", "{dry_res}");
    let preview = &dry_res["data"]["preview"];
    assert_eq!(
        preview["adoptedKeySignature"],
        json!({ "sharpsFlats": 2, "isMinor": false }),
        "{preview}"
    );
    assert_eq!(dry_res["data"]["stateUnchanged"], true);
    assert_eq!(dispatcher.domain().commit_count(), before_commits);
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert_eq!(
            project.key_signature, None,
            "dryRun 不得写 project.key_signature"
        );
    }

    // 2. 真实执行：响应如实回报，且调号确实进活跃工程。
    let real_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
        }),
    );
    assert_eq!(real_res["status"], "success", "{real_res}");
    let data = &real_res["data"];
    assert_eq!(data["applied"], true);
    assert_eq!(
        data["adoptedKeySignature"],
        json!({ "sharpsFlats": 2, "isMinor": false }),
        "{data}"
    );
    // 采纳的那条调号整类落地（`dropped == 0`），⛔ 不重复计成丢失。
    assert_eq!(data["losses"]["aux_events"]["key-signature"]["read"], 1);
    assert_eq!(data["losses"]["aux_events"]["key-signature"]["dropped"], 0);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert_eq!(
            project.key_signature,
            Some(yeban_model::project::KeySignature {
                sharps_flats: 2,
                is_minor: false,
            }),
            "调号必须在活跃工程生效"
        );
    }
    assert_eq!(dispatcher.domain().commit_count(), before_commits + 1);
    assert_ne!(project_bytes(&dispatcher), before_bytes);

    // 3. 撤销：一次 undo 完全还原，调号回到 None。
    let undo_res = call_tool(&mut dispatcher, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo_res["status"], "success", "{undo_res}");
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert_eq!(
            project.key_signature, None,
            "撤销后 project.key_signature 必须还原"
        );
    }

    let _ = fs::remove_file(smf_path);
}
const META_TITLE: &str = "夜半 · Imported Title";
const META_COPYRIGHT: &str = "(c) 2026 Yeban Test Author";

/// 造一份**带导体轨标题与版权声明**的 SMF（`Parallel`：track 0 是导体轨）。
fn create_metadata_smf_file() -> PathBuf {
    use yeban_midi::midi::{DEFAULT_PPQ, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
    let export = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2)],
        tracks: vec![MidiExportTrack {
            name: "Piano".to_string(),
            channel: 0,
            notes: vec![yeban_model::MidiNote::new(EntityId::new(), 0, 60, 480)],
            control_changes: Vec::new(),
        }],
        key_signature: None,
        title: Some(META_TITLE.to_string()),
        copyright: Some(META_COPYRIGHT.to_string()),
    };
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-import-metadata-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写带元数据的 SMF 文件");
    path
}

/// 一个**没有内容**且 `title` / `author` 均为空的工程（采纳门槛：三者全满足，
/// ⛔ 不覆盖已有值，非空工程更是一位都不动）。
fn project_without_identity() -> YebanProjectV1 {
    let mut project = yeban_midi::import::content_free_project();
    project.title = String::new();
    project.author = String::new();
    project
}

#[test]
fn test_import_midi_adopts_title_and_copyright() {
    let smf_path = create_metadata_smf_file();
    let mut dispatcher = dispatcher_with_project(project_without_identity());

    let before_bytes = project_bytes(&dispatcher);
    let before_commits = dispatcher.domain().commit_count();

    // 1. dryRun：预览如实报出将要采纳的两个值，工程一位都不动。
    let dry_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
            "dryRun": true,
        }),
    );
    assert_eq!(dry_res["status"], "success", "{dry_res}");
    let preview = &dry_res["data"]["preview"];
    assert_eq!(preview["adoptedTitle"], json!(META_TITLE), "{preview}");
    assert_eq!(preview["adoptedCopyright"], json!(META_COPYRIGHT));
    assert_eq!(dry_res["data"]["stateUnchanged"], true);
    assert_eq!(dispatcher.domain().commit_count(), before_commits);
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert!(project.title.is_empty(), "dryRun 不得写 project.title");
        assert!(project.author.is_empty(), "dryRun 不得写 project.author");
    }

    // 2. 真实执行：响应如实回报，且值确实进了活跃工程。
    let real_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
        }),
    );
    assert_eq!(real_res["status"], "success", "{real_res}");
    let data = &real_res["data"];
    assert_eq!(data["applied"], true);
    assert_eq!(data["adoptedTitle"], json!(META_TITLE), "{data}");
    assert_eq!(data["adoptedCopyright"], json!(META_COPYRIGHT));
    // 采纳的那条版权记入 attached（`dropped == 0`），⛔ 不重复计成丢失。
    assert_eq!(data["losses"]["aux_events"]["copyright"]["read"], 1);
    assert_eq!(data["losses"]["aux_events"]["copyright"]["dropped"], 0);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert_eq!(project.title, META_TITLE, "标题必须在活跃工程生效");
        assert_eq!(project.author, META_COPYRIGHT, "版权必须在活跃工程生效");
    }
    assert_eq!(dispatcher.domain().commit_count(), before_commits + 1);
    assert_ne!(project_bytes(&dispatcher), before_bytes);

    // 3. 撤销：一次 undo 完全还原，标题与署名回到空。
    let undo_res = call_tool(&mut dispatcher, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo_res["status"], "success", "{undo_res}");
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert!(project.title.is_empty(), "撤销后 project.title 必须还原");
        assert!(project.author.is_empty(), "撤销后 project.author 必须还原");
    }

    let _ = fs::remove_file(smf_path);
}

/// 造一份**格式 0**（单 `MTrk`）文件：唯一的轨名叫 `Piano`，同时带标题与版权元数据。
///
/// 关键性质：`TrackName` 类**只有一条事件**、且它被采纳成工程音轨名 ⇒ 该类 `dropped == 0`。
fn create_single_track_named_smf() -> PathBuf {
    use yeban_midi::midi::{DEFAULT_PPQ, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
    let export = MidiExport {
        format: MidiFormat::SingleTrack,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2)],
        tracks: vec![MidiExportTrack {
            name: "Piano".to_string(),
            channel: 0,
            notes: vec![yeban_model::MidiNote::new(EntityId::new(), 0, 60, 480)],
            control_changes: Vec::new(),
        }],
        key_signature: None,
        title: Some(META_TITLE.to_string()),
        copyright: Some(META_COPYRIGHT.to_string()),
    };
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-import-single-name-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写单轨 SMF 文件");
    path
}

/// 判据（⚠ 工具契约"未采纳的一律为 `null`"）：目标工程**已有标题／署名**时，
/// `adoptedTitle` / `adoptedCopyright` 必须是 `null`。
///
/// ⚠ 这一条钉的是一个**曾经存在**的谎报：旧实现对标题按 `TrackName` 类的
/// `dropped == 0` 回退读 `project.title` —— 当文件的轨名全部被采纳成**工程音轨名**、
/// 而工程本来就有标题时，响应会报"采纳了标题"（其实是工程自己的旧标题）。
#[test]
fn test_import_midi_never_reports_a_title_or_copyright_it_did_not_write() {
    let smf_path = create_single_track_named_smf();
    let project = project_with_notes(0);
    let original_title = project.title.clone();
    let original_author = project.author.clone();
    assert!(!original_title.is_empty(), "夹具前提：工程已有标题");
    let mut dispatcher = dispatcher_with_project(project);

    let res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({"path": smf_path.to_str().expect("str")}),
    );
    assert_eq!(res["status"], "success", "{res}");
    let data = &res["data"];
    assert_eq!(data["applied"], true, "{data}");
    // 控制臂：导入真的发生了（音符进来了），⛔ 整条路径没跑。
    assert_eq!(data["noteCount"], 1, "{data}");
    // 前提：轨名整类零丢失 —— 旧版正是在这里回退读 `project.title`。
    assert_eq!(
        data["losses"]["aux_events"]["track-name"]["read"], 1,
        "{data}"
    );
    assert_eq!(
        data["losses"]["aux_events"]["track-name"]["dropped"], 0,
        "{data}"
    );
    // 判据：⛔ 回报自己没写的标题 / 署名。
    assert_eq!(data["adoptedTitle"], Value::Null, "{data}");
    assert_eq!(data["adoptedCopyright"], Value::Null, "{data}");
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert_eq!(project.title, original_title, "工程标题 ⛔ 被导入改动");
        assert_eq!(project.author, original_author, "工程署名 ⛔ 被导入改动");
    }

    let _ = fs::remove_file(smf_path);
}

/// 造一份**带多点变速 / 拍号映射**的 SMF：tick 0 = 120 BPM + 4/4，
/// tick 3840 = 144 BPM + 3/4（源 PPQ 与工程同为 960 ⇒ tick 逐一对齐）。
fn create_tempo_meter_map_smf_file() -> PathBuf {
    use yeban_midi::midi::{DEFAULT_PPQ, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
    let mut late = MidiTempo::with_time_signature(144.0, 3, 2);
    late.tick = 3840;
    let export = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2), late],
        tracks: vec![MidiExportTrack {
            name: "Keys".to_string(),
            channel: 0,
            notes: vec![yeban_model::MidiNote::new(EntityId::new(), 0, 60, 480)],
            control_changes: Vec::new(),
        }],
        key_signature: None,
        title: None,
        copyright: None,
    };
    let bytes = export.to_smf_bytes().expect("SMF 编码");
    let path = std::env::temp_dir().join(format!(
        "yeban-test-import-tempomap-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写带变速/拍号映射的 SMF 文件");
    path
}

/// 判据（[MCP-TOOL-EXT-IMPORT-MIDI] / [ARCH-EXT-002]）：tick > 0 的变速与拍号映射点
/// 被采纳进 `project.tempo_map` / `project.time_signature_map`，响应逐条回报
/// `adoptedTempoMap` / `adoptedTimeSignatureMap`；dryRun 不落盘；一次 undo 完全还原。
#[test]
fn test_import_midi_adopts_tempo_map_and_time_signature_map() {
    let smf_path = create_tempo_meter_map_smf_file();
    let mut dispatcher = dispatcher_with_project(yeban_midi::import::content_free_project());

    let before_bytes = project_bytes(&dispatcher);
    let before_commits = dispatcher.domain().commit_count();

    // 144 BPM 经 mpqn（四舍五入到微秒）往返 ⇒ 143.9999...，按容差断言。
    let bpm_144_tolerance = 0.001;

    // 1. dryRun：预览如实报出将要采纳的两张映射点表，工程一位都不动。
    let dry_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
            "dryRun": true,
        }),
    );
    assert_eq!(dry_res["status"], "success", "{dry_res}");
    let preview = &dry_res["data"]["preview"];
    let preview_tempo = preview["adoptedTempoMap"]["3840"]
        .as_f64()
        .unwrap_or_else(|| panic!("dryRun 预览必须报出 tick 3840 的变速点: {preview}"));
    assert!(
        (preview_tempo - 144.0).abs() < bpm_144_tolerance,
        "预览变速点必须约 144 BPM, 实际 {preview_tempo}"
    );
    assert_eq!(
        preview["adoptedTimeSignatureMap"]["3840"],
        json!({ "numerator": 3, "denominator": 4 }),
        "{preview}"
    );
    assert_eq!(dry_res["data"]["stateUnchanged"], true);
    assert_eq!(dispatcher.domain().commit_count(), before_commits);
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert!(
            project.tempo_map.is_empty(),
            "dryRun 不得写 project.tempo_map"
        );
        assert!(
            project.time_signature_map.is_empty(),
            "dryRun 不得写 project.time_signature_map"
        );
    }

    // 2. 真实执行：映射点确实进活跃工程。
    let real_res = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({
            "path": smf_path.to_str().expect("str"),
        }),
    );
    assert_eq!(real_res["status"], "success", "{real_res}");
    let data = &real_res["data"];
    assert_eq!(data["applied"], true);
    let data_tempo = data["adoptedTempoMap"]["3840"]
        .as_f64()
        .unwrap_or_else(|| panic!("响应必须报出 tick 3840 的变速点: {data}"));
    assert!(
        (data_tempo - 144.0).abs() < bpm_144_tolerance,
        "响应变速点必须约 144 BPM, 实际 {data_tempo}"
    );
    assert_eq!(
        data["adoptedTimeSignatureMap"]["3840"],
        json!({ "numerator": 3, "denominator": 4 }),
        "{data}"
    );
    // 两类元事件整类读入且零丢失（采纳了，⛔ 不重复计成丢失）。
    assert_eq!(data["losses"]["aux_events"]["tempo"]["dropped"], 0);
    assert_eq!(data["losses"]["aux_events"]["time-signature"]["dropped"], 0);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        let adopted = project
            .tempo_map
            .get(&3840)
            .unwrap_or_else(|| panic!("tick 3840 变速点必须进 project.tempo_map"));
        assert!(
            (adopted - 144.0).abs() < bpm_144_tolerance,
            "project.tempo_map[3840] 必须约 144 BPM, 实际 {adopted}"
        );
        assert_eq!(
            project.time_signature_map.get(&3840),
            Some(&yeban_model::project::TimeSignature {
                numerator: 3,
                denominator: 4,
            }),
            "tick 3840 拍号点必须进 project.time_signature_map"
        );
    }
    assert_eq!(dispatcher.domain().commit_count(), before_commits + 1);
    assert_ne!(project_bytes(&dispatcher), before_bytes);

    // 3. 撤销：一次 undo 完全还原，两张映射回到空。
    let undo_res = call_tool(&mut dispatcher, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo_res["status"], "success", "{undo_res}");
    assert_eq!(project_bytes(&dispatcher), before_bytes);
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        assert!(
            project.tempo_map.is_empty(),
            "撤销后 project.tempo_map 必须还原"
        );
        assert!(
            project.time_signature_map.is_empty(),
            "撤销后 project.time_signature_map 必须还原"
        );
    }

    let _ = fs::remove_file(smf_path);
}
