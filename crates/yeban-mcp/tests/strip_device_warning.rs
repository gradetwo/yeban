//! **T-FIX-647 的判据**：第二台通道条设备被来源规则忽略时，编辑侧与分轨导出侧
//! 都必须响亮预警 —— 复用母带那条 `ignoredStripDevices` 判定（同一函数、同一形状），
//! 不是第二条规则。
//!
//! 判据全部走真实 `tools/call` 管线。夹具是 **2 条 MIDI 源轨（`Lead` / `Pad`）→ Master**，
//! 每条轨各一个带 1 个音符的片段（⇒ 有声，`deviceOnSilentTrack` 不应出现，警告只能
//! 来自通道条遮挡这一条原因）。
//!
//! | 判据 | 对象 | 读数 |
//! | :--- | :--- | :--- |
//! | [`the_first_insert_carries_no_strip_warning`] | 首台 EQ 的 `insertDevice` 响应 | `warnings[]` 为空 |
//! | [`the_second_insert_with_strip_params_warns_and_names_device_and_params`] | 再插压缩器（`threshold_db` + `ratio`）的响应 | 1 条 `ignoredStripDevices`，点名轨 / 槽位 1 / 设备 / 两参数；不阻断（提案/提交计数各 +1，可合并） |
//! | [`a_second_device_without_strip_params_does_not_warn`] | 再插混响（只有 `reverb_size`，非通道条族） | `warnings[]` 为空 |
//! | [`a_bypassed_second_device_does_not_warn`] | 再插旁通压缩器（带通道条参数） | `warnings[]` 为空（旁通不算被忽略） |
//! | [`dry_run_preview_carries_the_same_strip_warning`] | `dryRun:true` 的 `preview.warnings` | 与真实响应逐字相同 |
//! | [`removing_the_first_device_unmasks_the_second`] | 删首台 EQ 后 | `warnings[]` 为空（压缩器转正为来源） |
//! | [`a_third_insert_names_both_ignored_devices`] | 再插第三台（带 `threshold_db`） | 1 条警告，`count` 2，`devices[]` 点名槽位 1 与 2 |
//! | [`export_stems_carries_the_strip_warning_on_the_affected_row`] | `dryRun` 分轨导出 | `Lead` 行有 `stripWarning`（同一 `code` / 形状），`Pad` 行没有这个键 |
//! | [`export_stems_real_write_agrees_with_dry_run`] | 真实落盘 | `stripWarning` 与预览逐字相同；`sha256` / `bytes` 与预览相同（警告只读不改字节） |
//! | [`export_stems_clean_project_has_no_strip_warning`] | 干净工程的分轨导出 | 没有任何一行带 `stripWarning` |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test strip_device_warning -- --nocapture`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};

/// 确定性 ULID（末 4 位是种子；`01J8ZR` + 16 个 `0` + 4 位种子 = 26 字符）。
fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// `Lead`（有声）+ `Pad`（有声）→ `Master` 的夹具；`with_devices` 为真时 `Lead`
/// 的设备链预置 `[EQ, 压缩器]`（分轨判据用，免走两次编辑调用）。
struct Fixture {
    project: YebanProjectV1,
    lead: EntityId,
    clip: EntityId,
}

fn fixture(with_devices: bool) -> Fixture {
    let master = id(1);
    let lead = id(2);
    let pad = id(3);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "strip-647".to_owned(),
        bpm: 120.0,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    for (index, name) in ["Lead", "Pad"].iter().enumerate() {
        let seed = u32::try_from(index).expect("夹具规模放得进 u32");
        let track = id(2 + seed);
        let clip = id(10 + seed);
        let placement = id(20 + seed);
        let note = id(30 + seed);
        let edge = id(40 + seed);
        let mut params = TrackV3 {
            id: track,
            name: (*name).to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        };
        params.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: 1_920,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
                fade_in_ticks: None,
                fade_out_ticks: None,
                source_start_samples: None,
            },
        );
        let mut notes = BTreeMap::new();
        notes.insert(
            note,
            MidiNote::new(note, 0, 48 + u8::try_from(index * 12).expect("取模"), 1_920),
        );
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: format!("{name}Clip"),
                content: ClipContent::Midi { notes },
            },
        );
        project.tracks.insert(track, params);
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
                source_lane: None,
                dest_lane: None,
            },
        );
    }
    project.routing_graph.nodes = vec![lead, pad, master];
    if with_devices {
        project
            .tracks
            .get_mut(&lead)
            .expect("Lead")
            .devices
            .extend([eq_device(), compressor_device(false)]);
    }
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    Fixture {
        project,
        lead,
        clip: id(10),
    }
}

/// 首台设备：EQ，带一个通道条参数 `eq_low_gain`。
fn eq_device() -> DeviceDefinition {
    DeviceDefinition {
        id: id(50),
        name: "EQ".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![ParameterValue {
            name: "eq_low_gain".to_owned(),
            value: 6.0,
            unit: Some("dB".to_owned()),
        }],
        latency_samples: 0,
        plugin: None,
    }
}

/// 第二台设备：压缩器，带两个通道条参数 `threshold_db` / `ratio`。
fn compressor_device(bypassed: bool) -> DeviceDefinition {
    DeviceDefinition {
        id: id(51),
        name: "Compressor".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed,
        params: vec![
            ParameterValue {
                name: "threshold_db".to_owned(),
                value: -18.0,
                unit: Some("dB".to_owned()),
            },
            ParameterValue {
                name: "ratio".to_owned(),
                value: 4.0,
                unit: None,
            },
        ],
        latency_samples: 0,
        plugin: None,
    }
}

/// 第三台设备：另一台压缩器，只带一个通道条参数。
fn second_compressor_device() -> DeviceDefinition {
    DeviceDefinition {
        id: id(53),
        name: "Compressor 2".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![ParameterValue {
            name: "threshold_db".to_owned(),
            value: -24.0,
            unit: Some("dB".to_owned()),
        }],
        latency_samples: 0,
        plugin: None,
    }
}

/// 不带通道条参数的第二台设备：混响，只有 `reverb_size`（混响族）。
fn reverb_device() -> DeviceDefinition {
    DeviceDefinition {
        id: id(52),
        name: "Reverb".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![ParameterValue {
            name: "reverb_size".to_owned(),
            value: 0.5,
            unit: None,
        }],
        latency_samples: 0,
        plugin: None,
    }
}

/// `DeviceDefinition` → `insertDevice` 的 `device` 载荷（工具面 camelCase）。
fn device_json(device: &DeviceDefinition) -> Value {
    json!({
        "deviceId": device.id.to_canonical_string(),
        "name": device.name,
        "kind": "InternalEffect",
        "bypassed": device.bypassed,
        "latencySamples": device.latency_samples,
        "params": device.params.iter().map(|param| {
            let mut item = json!({"name": param.name, "value": param.value});
            if let Some(unit) = &param.unit {
                item["unit"] = json!(unit);
            }
            item
        }).collect::<Vec<_>>(),
    })
}

fn dispatcher(project: &YebanProjectV1) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-strip-647.yeban"),
            project.clone(),
            false,
        )
        .expect("注入夹具工程");
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "strip-647",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

/// 一次 `insertDevice` 调用（顶层 `trackId` / `clipId` 指向有声的 `Lead`）。
fn insert_device_on(
    dispatcher: &mut Dispatcher,
    auth: &str,
    fixture: &Fixture,
    device: &DeviceDefinition,
    dry_run: bool,
) -> Value {
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({
            "trackId": fixture.lead.to_canonical_string(),
            "clipId": fixture.clip.to_canonical_string(),
            "dryRun": dry_run,
            "ops": [{
                "kind": "insertDevice",
                "trackId": fixture.lead.to_canonical_string(),
                "device": device_json(device),
            }],
        }),
    )
}

/// 合并一条提案（返回合并响应；调用方自断言 `status`）。
fn merge(dispatcher: &mut Dispatcher, auth: &str, proposal_id: &str) -> Value {
    call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "strip-647 判据合并"}),
    )
}

/// 插一台设备并合并（返回插的响应）；合并失败直接拍在断言里。
fn insert_and_merge(
    dispatcher: &mut Dispatcher,
    auth: &str,
    fixture: &Fixture,
    device: &DeviceDefinition,
) -> Value {
    let written = insert_device_on(dispatcher, auth, fixture, device, false);
    assert_eq!(written["status"], "success", "{written}");
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = merge(dispatcher, auth, &proposal_id);
    assert_eq!(merged["status"], "success", "{merged}");
    written
}

/// 从一次 `insertDevice` 响应里取出 `warnings[]`（必须是数组）。
fn warnings_of(response: &Value) -> Vec<Value> {
    response["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组")
        .clone()
}

/// 在 `warnings[]` 里找 `code` 为 `ignoredStripDevices` 的那条（0 条或 1 条）。
fn strip_warning(warnings: &[Value]) -> Option<&Value> {
    let found: Vec<&Value> = warnings
        .iter()
        .filter(|warning| warning["code"] == "ignoredStripDevices")
        .collect();
    assert!(
        found.len() <= 1,
        "同一轨的遮挡预警必须至多一条: {warnings:?}"
    );
    found.into_iter().next()
}

/// 判据 ①：首台 EQ 的插入不报（来源本身不算被忽略；有声轨也不触发静音轨警告）。
#[test]
fn the_first_insert_carries_no_strip_warning() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, &fixture, &eq_device(), false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "首台设备不该有任何警告: {written}"
    );
}

/// 判据 ②：第二台压缩器（带通道条参数）⇒ 1 条 `ignoredStripDevices`，点名轨 /
/// 槽位 / 设备 / 被忽略参数；且不阻断（提案与提交计数各 +1，合并后设备在链上）。
#[test]
fn the_second_insert_with_strip_params_warns_and_names_device_and_params() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    let proposals_before = dispatcher.domain().proposal_count();
    let commits_before = dispatcher.domain().commit_count();

    let written = insert_device_on(
        &mut dispatcher,
        &auth,
        &fixture,
        &compressor_device(false),
        false,
    );
    assert_eq!(written["status"], "success", "{written}");
    let warnings = warnings_of(&written);
    let warning = strip_warning(&warnings).expect("第二台设备必须出遮挡预警");
    assert_eq!(warning["severity"], "warning", "{warning}");
    assert_eq!(warning["blocking"], false, "{warning}");
    assert_eq!(warning["count"], 1, "{warning}");
    assert_eq!(warning["devices"][0]["track"], "Lead", "{warning}");
    assert_eq!(warning["devices"][0]["slot"], 1, "{warning}");
    assert_eq!(warning["devices"][0]["device"], "Compressor", "{warning}");
    assert_eq!(
        warning["devices"][0]["ignoredParams"],
        json!(["threshold_db", "ratio"]),
        "{warning}"
    );
    assert!(
        warning["hint"]
            .as_str()
            .expect("hint 是字符串")
            .contains("首台设备"),
        "修法提示必须点名首台设备: {warning}"
    );

    // 不阻断：两次计数各 +1，且合并成功。
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before + 1,
        "提案计数必须照常 +1"
    );
    assert_eq!(
        dispatcher.domain().commit_count(),
        commits_before + 1,
        "提案提交必须照常 +1"
    );
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = merge(&mut dispatcher, &auth, &proposal_id);
    assert_eq!(merged["status"], "success", "{merged}");
}

/// 判据 ③：第二台设备只有非通道条参数（混响族 `reverb_size`）⇒ 不报。
#[test]
fn a_second_device_without_strip_params_does_not_warn() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    let written = insert_device_on(&mut dispatcher, &auth, &fixture, &reverb_device(), false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "无通道条参数的第二台设备不该报: {written}"
    );
}

/// 判据 ④：第二台设备被旁通（哪怕带通道条参数）⇒ 不报（旁通本来就不进信号链）。
#[test]
fn a_bypassed_second_device_does_not_warn() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    let written = insert_device_on(
        &mut dispatcher,
        &auth,
        &fixture,
        &compressor_device(true),
        false,
    );
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "旁通的第二台设备不该报: {written}"
    );
}

/// 判据 ⑤：`dryRun` 预览的警告与真实响应逐字相同。
#[test]
fn dry_run_preview_carries_the_same_strip_warning() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    let proposals_before = dispatcher.domain().proposal_count();

    let planned = insert_device_on(
        &mut dispatcher,
        &auth,
        &fixture,
        &compressor_device(false),
        true,
    );
    assert_eq!(planned["status"], "success", "{planned}");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before,
        "dryRun 不得创建提案"
    );
    let preview_warnings = planned["data"]["preview"]["warnings"]
        .as_array()
        .expect("preview.warnings 必须是数组");
    assert!(
        strip_warning(preview_warnings).is_some(),
        "dryRun 预览必须带遮挡预警: {preview_warnings:?}"
    );

    let written = insert_device_on(
        &mut dispatcher,
        &auth,
        &fixture,
        &compressor_device(false),
        false,
    );
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        planned["data"]["preview"]["warnings"], written["data"]["warnings"],
        "dryRun 与真实响应的 warnings 必须逐字相同"
    );
}

/// 判据 ⑥：删掉首台 EQ ⇒ 压缩器转正为来源 ⇒ 不再报（`RemoveDevice` 同样走判定）。
#[test]
fn removing_the_first_device_unmasks_the_second() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    insert_and_merge(&mut dispatcher, &auth, &fixture, &compressor_device(false));
    let removed = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": fixture.lead.to_canonical_string(),
            "clipId": fixture.clip.to_canonical_string(),
            "dryRun": false,
            "ops": [{
                "kind": "removeDevice",
                "trackId": fixture.lead.to_canonical_string(),
                "deviceId": id(50).to_canonical_string(),
            }],
        }),
    );
    assert_eq!(removed["status"], "success", "{removed}");
    assert_eq!(
        removed["data"]["warnings"],
        json!([]),
        "删首台后压缩器已是来源，不该再报: {removed}"
    );
}

/// 判据 ⑦：第三台（带通道条参数）⇒ 同一条警告 `count` 2，`devices[]` 点名槽位 1 与 2。
#[test]
fn a_third_insert_names_both_ignored_devices() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    insert_and_merge(&mut dispatcher, &auth, &fixture, &eq_device());
    insert_and_merge(&mut dispatcher, &auth, &fixture, &compressor_device(false));
    let written = insert_device_on(
        &mut dispatcher,
        &auth,
        &fixture,
        &second_compressor_device(),
        false,
    );
    assert_eq!(written["status"], "success", "{written}");
    let warnings = warnings_of(&written);
    let warning = strip_warning(&warnings).expect("第三台设备必须出遮挡预警");
    assert_eq!(warning["count"], 2, "{warning}");
    assert_eq!(warning["devices"][0]["slot"], 1, "{warning}");
    assert_eq!(warning["devices"][0]["device"], "Compressor", "{warning}");
    assert_eq!(warning["devices"][1]["slot"], 2, "{warning}");
    assert_eq!(warning["devices"][1]["device"], "Compressor 2", "{warning}");
    assert_eq!(
        warning["devices"][1]["ignoredParams"],
        json!(["threshold_db"]),
        "{warning}"
    );
}

// ---------------------------------------------------------------------------
// 分轨判据：复用 `export_stems.rs` 的临时目录与调用形态
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一个临时目录，`Drop` 时恢复权限并删除。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-strip-647-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    /// 工程文件落点（与分轨目录分开）。
    fn project_path(&self) -> PathBuf {
        let dir = self.dir.join("project");
        fs::create_dir_all(&dir).expect("建工程目录");
        dir.join("demo.yebanproject")
    }

    /// 分轨输出目录。
    fn stems_dir(&self) -> PathBuf {
        let dir = self.dir.join("stems");
        fs::create_dir_all(&dir).expect("建分轨目录");
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn stems_dispatcher(project: &YebanProjectV1, path: &Path) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path, project.clone(), false)
        .expect("注入夹具工程");
    (dispatcher, auth)
}

/// 一次 `yeban_export_stems` 调用，返回**装 stems 的那层**：`dryRun` 时是协议层包络
/// 里的 `data.preview`（工具自己的实测预览住在那里，口径先例见 `export_stems.rs`），
/// 真实落盘时是 `data` 本身（失败直接把响应拍在断言里）。
fn export_stems(
    dispatcher: &mut Dispatcher,
    auth: &str,
    out_dir: &std::path::Path,
    dry_run: bool,
) -> Value {
    let mut arguments = json!({"outDir": out_dir.display().to_string()});
    if dry_run {
        arguments["dryRun"] = json!(true);
    }
    let response = call(dispatcher, auth, "yeban_export_stems", arguments);
    assert_eq!(response["status"], "success", "导出必须成功: {response}");
    if dry_run {
        assert_eq!(
            response["data"]["dryRun"],
            json!(true),
            "包络必须自述这是干跑"
        );
        assert_eq!(
            response["data"]["stateUnchanged"],
            json!(true),
            "包络必须自述状态未变"
        );
        response["data"]["preview"].clone()
    } else {
        response["data"].clone()
    }
}

/// 在 `stems[]` 里按轨名找行。
fn stem_row<'a>(data: &'a Value, name: &str) -> &'a Value {
    data["stems"]
        .as_array()
        .expect("stems 必须是数组")
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("找不到 {name} 的分轨行: {data}"))
}

/// 判据 ⑧：`dryRun` 分轨导出 ⇒ `Lead` 行带 `stripWarning`（与母带同一 `code` /
/// 同一形状），`Pad` 行没有这个键。
#[test]
fn export_stems_carries_the_strip_warning_on_the_affected_row() {
    let fixture = fixture(true);
    let scratch = Scratch::new("dry-run");
    let (mut dispatcher, auth) = stems_dispatcher(&fixture.project, &scratch.project_path());
    let data = export_stems(&mut dispatcher, &auth, &scratch.stems_dir(), true);
    let lead = stem_row(&data, "Lead");
    let warning = lead
        .get("stripWarning")
        .expect("Lead 行必须带 stripWarning");
    assert_eq!(warning["code"], "ignoredStripDevices", "{warning}");
    assert_eq!(warning["severity"], "warning", "{warning}");
    assert_eq!(warning["blocking"], false, "{warning}");
    assert_eq!(warning["count"], 1, "{warning}");
    assert_eq!(warning["devices"][0]["track"], "Lead", "{warning}");
    assert_eq!(warning["devices"][0]["slot"], 1, "{warning}");
    assert_eq!(warning["devices"][0]["device"], "Compressor", "{warning}");
    assert_eq!(
        warning["devices"][0]["ignoredParams"],
        json!(["threshold_db", "ratio"]),
        "{warning}"
    );
    assert!(
        !stem_row(&data, "Pad")
            .as_object()
            .expect("Pad 行是对象")
            .contains_key("stripWarning"),
        "干净的 Pad 行必须没有 stripWarning: {data}"
    );
}

/// 判据 ⑨：真实落盘的 `stripWarning` 与预览逐字相同，且 `sha256` / `bytes` 与预览
/// 相同 —— 警告只读不改字节（与母带削波预警同一条纪律）。
#[test]
fn export_stems_real_write_agrees_with_dry_run() {
    let fixture = fixture(true);
    let scratch = Scratch::new("real-write");
    let (mut dispatcher, auth) = stems_dispatcher(&fixture.project, &scratch.project_path());
    let preview = export_stems(&mut dispatcher, &auth, &scratch.stems_dir(), true);
    let written = export_stems(&mut dispatcher, &auth, &scratch.stems_dir(), false);
    let preview_lead = stem_row(&preview, "Lead");
    let written_lead = stem_row(&written, "Lead");
    // 先钉存在：否则下面那条"一致"在两边同缺时也会空通过（红控 B 曾抓到这条）。
    assert_eq!(
        written_lead["stripWarning"]["code"], "ignoredStripDevices",
        "真实落盘必须带遮挡预警: {written_lead}"
    );
    assert_eq!(
        preview_lead["stripWarning"], written_lead["stripWarning"],
        "dryRun 与真实落盘的 stripWarning 必须逐字相同"
    );
    assert_eq!(
        preview_lead["sha256"], written_lead["sha256"],
        "警告不得改动音频字节"
    );
    assert_eq!(
        preview_lead["bytes"], written_lead["bytes"],
        "警告不得改动音频字节数"
    );
    // 文件真的落了盘。
    let path = written_lead["path"].as_str().expect("path 是字符串");
    assert!(fs::metadata(path).is_ok(), "分轨文件必须已落盘: {path}");
}

/// 判据 ⑩：干净工程的分轨导出 ⇒ 没有任何一行带 `stripWarning`。
#[test]
fn export_stems_clean_project_has_no_strip_warning() {
    let fixture = fixture(false);
    let scratch = Scratch::new("clean");
    let (mut dispatcher, auth) = stems_dispatcher(&fixture.project, &scratch.project_path());
    let data = export_stems(&mut dispatcher, &auth, &scratch.stems_dir(), true);
    for row in data["stems"].as_array().expect("stems 必须是数组") {
        assert!(
            !row.as_object()
                .expect("行是对象")
                .contains_key("stripWarning"),
            "干净工程的分轨行必须没有 stripWarning: {row}"
        );
    }
}
