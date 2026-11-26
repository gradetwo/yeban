//! **T-FIX-500**：MCP 打开/保存路径的悬空音频资产核对。
//!
//! 对照面是 `yeban-app`：`open.rs` 打开即给 `AssetAudit` 警告，`save.rs`
//! 在写盘前用 `missing_audio_assets` 拒绝缺件保存。MCP 原先两条路径都不查，
//! 于是"GUI 拒绝保存的容器"可以被 MCP 原样存回去（T-FIX-329/330 的口径不对称）。
//!
//! 判据按"能变红"写：缺件工程**打开必须点名**、**保存必须响亮拒绝且原文件
//! 逐字节不变**；干净工程保存仍必须成功（防止核对过宽把所有保存打死）。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::project::{ClipContent, ClipPoolEntry};
use yeban_model::{AssetHash, CommitGraph, EntityId, YebanProjectV1};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-audit-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    assert_eq!(outcome.http_status, 200, "{name} 必须是带内结果");
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

fn container(project: &YebanProjectV1) -> Vec<u8> {
    let history = serde_json::to_vec(&CommitGraph::new()).expect("空图谱 JSON");
    yeban_model::container::write_project_container(project, &history, &BTreeMap::new())
        .expect("写真容器")
}

/// `filled_project()` 自带一个 Audio 片段，但其夹具容器不带资产字节 —— 先剪掉
/// 所有 Audio 条目与摆放，得到一份"确实干净"的对照工程。
fn project_without_audio() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    project
        .clip_pool
        .retain(|_, entry| !matches!(entry.content, ClipContent::Audio { .. }));
    let remaining: Vec<EntityId> = project.clip_pool.keys().copied().collect();
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| remaining.contains(&placement.clip_id));
    }
    project.validate().expect("对照工程自身必须合法");
    project
}

/// 一份合法工程 + 一个 `Audio` 片段指向**没有任何字节**的哈希。
fn dangling_project() -> (YebanProjectV1, AssetHash) {
    let mut project = project_without_audio();
    let asset = AssetHash::of_bytes(b"t-fix-500-missing-audio-bytes");
    let id = EntityId::new();
    project.clip_pool.insert(
        id,
        ClipPoolEntry {
            id,
            name: "悬空音频片段".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );
    project.validate().expect("负样本工程自身必须通过 validate");
    (project, asset)
}

#[test]
fn open_names_dangling_audio_assets_and_save_refuses_without_writing() {
    let scratch = Scratch::new("dangling");
    let (project, asset) = dangling_project();
    let bytes = container(&project);
    let path = scratch.dir.join("dangling.yeban");
    fs::write(&path, &bytes).expect("写容器");

    let (mut dispatcher, auth) = dispatcher();
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(opened["data"]["opened"], true, "{opened}");
    assert_eq!(
        opened["data"]["missingAudioAssetCount"], 1,
        "打开回执必须点名悬空资产: {opened}"
    );
    let listed = opened["data"]["missingAudioAssets"]
        .as_array()
        .expect("missingAudioAssets 必须是数组");
    assert_eq!(listed.len(), 1, "{opened}");
    assert_eq!(listed[0]["asset"], asset.as_str(), "{opened}");

    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["status"], "error", "缺件保存必须拒绝: {saved}");
    assert_eq!(saved["error"]["code"], "IO_ERROR", "{saved}");
    assert_eq!(
        saved["error"]["data"]["reason"], "missingAudioAsset",
        "{saved}"
    );
    assert_eq!(
        saved["error"]["data"]["missingAssets"][0],
        asset.as_str(),
        "{saved}"
    );
    assert_eq!(
        fs::read(&path).expect("原文件必须还在"),
        bytes,
        "拒绝保存时连一个字节都不许写"
    );
}

#[test]
fn a_clean_project_still_saves_and_reports_zero_missing_assets() {
    let scratch = Scratch::new("clean");
    let project = project_without_audio();
    let bytes = container(&project);
    let path = scratch.dir.join("clean.yeban");
    fs::write(&path, &bytes).expect("写容器");

    let (mut dispatcher, auth) = dispatcher();
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(opened["data"]["missingAudioAssetCount"], 0, "{opened}");

    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({"force": true}),
    );
    assert_eq!(saved["status"], "success", "干净工程必须仍能保存: {saved}");
    assert_eq!(saved["data"]["saved"], true, "{saved}");
}
