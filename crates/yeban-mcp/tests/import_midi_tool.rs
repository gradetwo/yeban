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
//!    （`adoptedTempoMap` / `adoptedTimeSignatureMap`、dryRun 不落盘、undo 还原）；
//! 6. ⭐ [ARCH-EXT-002]：**两工具闭环** —— `yeban_export_midi` 回传的字节（base64 解回）
//!    写成 `.mid` 后再由 `yeban_import_midi` 导入，音符四元组与标题／署名逐字往返。
//! 7. ⭐ [ARCH-EXT-002]：歌词五桶 `lyricCounts` —— `nonUtf8` / `empty` 两个丢失桶
//!    与 `read`/`attached`/`dropped` 一起上报（手搓含空/坏 UTF-8 的 `FF 05`）。

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};
use yeban_mcp::tools::{self, ErrorCode, SideEffect, TOOL_COUNT};
use yeban_mcp::undo_session::canonical_project_bytes;
use yeban_model::samples::{filled_project, project_with_notes};
use yeban_model::{ClipContent, EntityId, MidiNote, YebanProjectV1};

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

// ---------------------------------------------------------------------------
// ⭐ 两工具闭环：`yeban_export_midi` 的字节 ⇒ `yeban_import_midi` 的工程状态。
// ---------------------------------------------------------------------------

/// 测试侧的**独立** base64 解码器（RFC 4648 §4）—— 与 `extension_tools.rs` 同款，
/// 与被测编码器分开写（不共用被测量的实现）。
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

/// 判据（`[ARCH-EXT-002]`）：**导出工具**回传的字节喂给**导入工具**后，
/// 音符四元组与标题／署名逐字往返 —— 两个工具之间没有"只有单元测试知道"的暗沟。
#[test]
fn test_export_tool_bytes_reimported_through_the_import_tool() {
    const TITLE: &str = "夜半 · 工具闭环";
    const COPYRIGHT: &str = "(c) 2026 Yeban Loop";

    // 1. 源工程：空工程骨架 + 两颗音符 + 标题 + 署名。
    let mut source = yeban_midi::import::content_free_project();
    source.title = TITLE.to_owned();
    source.author = COPYRIGHT.to_owned();
    let clip_id = source
        .clip_pool
        .iter()
        .find_map(|(id, entry)| matches!(&entry.content, ClipContent::Midi { .. }).then_some(*id))
        .expect("content-free 夹具必须含 MIDI 片段");
    {
        let entry = source.clip_pool.get_mut(&clip_id).expect("片段在池里");
        let ClipContent::Midi { notes } = &mut entry.content else {
            unreachable!("刚按 MIDI 变体筛过");
        };
        let first = MidiNote::new(EntityId::new(), 0, 60, 480);
        notes.insert(first.id, first);
        let mut second = MidiNote::new(EntityId::new(), 960, 67, 240);
        second.velocity = 64;
        notes.insert(second.id, second);
    }
    let mut exporter = dispatcher_with_project(source);

    let exported = call_tool(&mut exporter, "yeban_export_midi", json!({}));
    assert_eq!(exported["status"], "success", "{exported}");
    assert_eq!(
        exported["data"]["notes"],
        json!(2),
        "控制臂：两颗音符都进了 SMF: {exported}"
    );
    let bytes = decode_base64(
        exported["data"]["content"]
            .as_str()
            .expect("导出响应必须带 base64 正文"),
    );
    assert_eq!(
        u64::try_from(bytes.len()).expect("长度"),
        exported["data"]["bytes"].as_u64().expect("bytes 读数"),
        "解回来的字节长度必须等于响应自报的 bytes: {exported}"
    );

    // 2. 字节写成 `.mid`，由**导入工具**导入一个没有身份的空工程。
    let smf_path = std::env::temp_dir().join(format!(
        "yeban-test-tool-loop-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&smf_path, &bytes).expect("写 SMF 文件");

    let mut importer = dispatcher_with_project(project_without_identity());
    let imported = call_tool(
        &mut importer,
        "yeban_import_midi",
        json!({"path": smf_path.to_str().expect("str")}),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    let data = &imported["data"];
    assert_eq!(data["applied"], true, "{data}");
    assert_eq!(data["noteCount"], json!(2), "{data}");
    assert_eq!(data["adoptedTitle"], json!(TITLE), "{data}");
    assert_eq!(data["adoptedCopyright"], json!(COPYRIGHT), "{data}");
    {
        let project = importer.domain().active_project().expect("活跃工程");
        assert_eq!(project.title, TITLE, "标题必须在活跃工程生效");
        assert_eq!(project.author, COPYRIGHT, "署名必须在活跃工程生效");
        let mut notes: Vec<(u64, u8, u64, u8)> = project
            .clip_pool
            .values()
            .filter_map(|entry| match &entry.content {
                ClipContent::Midi { notes } => Some(notes),
                ClipContent::Audio { .. } => None,
            })
            .flat_map(|notes| notes.values())
            .map(|note| {
                (
                    note.start_tick,
                    note.pitch,
                    note.duration_ticks,
                    note.velocity,
                )
            })
            .collect();
        notes.sort_unstable();
        assert_eq!(
            notes,
            vec![(0, 60, 480, 100), (960, 67, 240, 64)],
            "音符四元组必须逐条往返"
        );
    }

    let _ = fs::remove_file(smf_path);
}

/// 手搓一份格式 0 / 960 PPQ 的 SMF：tick 0 一颗 C4（vel 100、dur 960），
/// 加**三条** `FF 05` —— `do`（tick 0，可归属候选）、空负载（`empty` 桶）、
/// 非 UTF-8 负载 `FF FE`（`nonUtf8` 桶）。后两条编码器不会产出，只能手搓
/// （同 `yeban-midi` 的 `parser_counts_empty_and_non_utf8_lyrics_without_guessing`）。
fn create_lyric_bucket_smf_file() -> PathBuf {
    let payload: Vec<u8> = vec![
        0x00, 0x90, 0x3C, 0x64, // NoteOn C4 vel 100 @ tick 0
        0x00, 0xFF, 0x05, 0x02, b'd', b'o', // FF 05 "do" @ tick 0
        0x87, 0x40, 0x80, 0x3C, 0x00, // NoteOff @ tick 960
        0x00, 0xFF, 0x05, 0x00, // 空 FF 05
        0x00, 0xFF, 0x05, 0x02, 0xFF, 0xFE, // 非 UTF-8 FF 05
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // 格式 0
    bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
    bytes.extend_from_slice(&960u16.to_be_bytes());
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("测试负载很短")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&payload);
    let path = std::env::temp_dir().join(format!(
        "yeban-mcp-import-lyric-buckets-{}-{}.mid",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    fs::write(&path, bytes).expect("写手搓 SMF");
    path
}

/// 判据（[MCP-TOOL-EXT-IMPORT-MIDI]）：歌词丢失的 `nonUtf8` / `empty` 两桶**必须在
/// `lyricCounts` 里**。`read == attached + dropped` 只覆盖候选桶 —— 一个全是坏歌词的
/// 文件会报 `read=0`，⛔ 这不代表文件里没有 `FF 05`；两桶此前只在 CLI/GUI 行可见，
/// MCP 调用方收到的是"四个键里两个重复"的视图（`orphan` 与 `dropped` 同值）。
#[test]
fn test_import_midi_reports_empty_and_non_utf8_lyric_buckets() {
    let smf_path = create_lyric_bucket_smf_file();
    let mut dispatcher = dispatcher_with_project(yeban_midi::import::content_free_project());

    let response = call_tool(
        &mut dispatcher,
        "yeban_import_midi",
        json!({ "path": smf_path.to_str().expect("str") }),
    );
    assert_eq!(response["status"], "success", "{response}");
    let data = &response["data"];
    assert_eq!(data["noteCount"], json!(1), "{data}");
    assert_eq!(
        data["lyricCounts"],
        json!({
            "read": 1,
            "attached": 1,
            "orphan": 0,
            "dropped": 0,
            "nonUtf8": 1,
            "empty": 1,
        }),
        "五个桶逐个点名 —— 响应里 ⛔ 不能少 nonUtf8/empty: {data}"
    );

    // 可归属的那条真成了音节；两个丢失桶 ⛔ 不伪造文本（工程里只有这一颗音符）。
    {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        let syllables: Vec<Option<&str>> = project
            .clip_pool
            .values()
            .filter_map(|entry| match &entry.content {
                ClipContent::Midi { notes } => Some(notes),
                ClipContent::Audio { .. } => None,
            })
            .flat_map(|notes| notes.values())
            .map(|note| note.syllable.as_deref())
            .collect();
        assert_eq!(syllables, vec![Some("do")], "只有可归属的一条成为音节");
    }

    let _ = fs::remove_file(smf_path);
}
