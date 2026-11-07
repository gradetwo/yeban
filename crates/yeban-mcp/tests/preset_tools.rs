//! 夜半原生预制机制（设计稿 `yeban-preset-design.md`）的 MCP 工具判据：
//! `yeban_list_presets` / `yeban_apply_preset` / `yeban_save_preset` /
//! `yeban_preset_bypass` 四个工具经真实 `tools/call` 管线。
//!
//! | 判据 | 量 | 单位 |
//! | :--- | :--- | :--- |
//! | 系统清单 | 工厂预制数 | 6 件 |
//! | 未知 kind | 错误码 | `INVALID_PARAMETER_RANGE` |
//! | 应用预制（strip+effects） | `Preset:` 设备数 / 采样器 | 2 件 / 不变 |
//! | 应用预制（全类） | 采样器路径 | 含 `Bowed` |
//! | 未知预制 | 错误码 | `ENTITY_NOT_FOUND` |
//! | 旁通 | 旁通后设备数 / 取消后设备数 | 0 件 / 恢复 |
//! | 自动加载 | 绑定采样器后 `preset_id` | 钢琴预制标识 |
//! | 另存 | 用户库文件 / 重名 | 落盘 / `CONFLICT` |
//! | 保存收据 | 内嵌预制数（带出 / 不带出） | 1 / 0 |

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

const PIANO_PRESET: &str = "sys:salamander-grand:piano-warm-room";
const CELLO_PRESET: &str = "sys:karoryfer-bigcat-cello:cello-warm-hall";
const CELLO_SFZ_FRAGMENT: &str = "Bowed";
const PIANO_SFZ_FRAGMENT: &str = "Salamander Grand Piano V3.sfz";

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-preset-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `$HOME` 是进程级状态 ⇒ 改它的判据串行持 `ENV_LOCK`，用 RAII 守卫恢复原值
///（与 `load_sfz_relative_path.rs` 的 `LibraryEnv` 同一口径）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 把 `$HOME` 指到临时目录（用户预制库随之搬家），析构时恢复。
struct HomeGuard {
    old: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HomeGuard {
    fn redirect(dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let old = std::env::var("HOME").ok();
        // SAFETY: 持 ENV_LOCK；本测试进程内没有其它线程在此时改 `$HOME`。
        unsafe {
            std::env::set_var("HOME", dir);
        }
        Self { old, _lock: lock }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // SAFETY: 与 `redirect` 同一条理由；恢复的是进入时读到的原值。
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

fn user_library_dir(home: &Path) -> PathBuf {
    home.join("Music").join("Audio Music Apps").join("Patches")
}

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-preset-{}.yeban", std::process::id())),
            filled_project(),
            false,
        )
        .expect("注入规范工程");
    dispatcher
}

/// 走真实 `tools/call` 管线，不预设成败（错误断言用它）。
fn call_any(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
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
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 成功路径：`status` 必须是 `success`，回 `data`。
fn call(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let result = call_any(dispatcher, name, arguments);
    assert_eq!(result["status"], "success", "`{name}` 工具层失败: {result}");
    result["data"].clone()
}

/// 失败路径：`status` 必须是 `error`，错误码逐字。
fn call_expect_error(dispatcher: &mut Dispatcher, name: &str, arguments: Value, code: &str) {
    let result = call_any(dispatcher, name, arguments);
    assert_eq!(
        result["status"], "error",
        "`{name}` 应当工具层失败: {result}"
    );
    assert_eq!(result["error"]["code"], code, "{result}");
}

/// 按轨名取轨身份（`filled_project`：Lead / Bass / Aux Reverb）。
fn track_id(dispatcher: &Dispatcher, name: &str) -> String {
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .tracks
        .values()
        .find(|track| track.name == name)
        .unwrap_or_else(|| panic!("轨 `{name}` 不在规范工程里"))
        .id
        .to_canonical_string()
}

/// 按轨名取轨快照（克隆）。
fn track_snapshot(dispatcher: &Dispatcher, name: &str) -> yeban_model::TrackV3 {
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .tracks
        .values()
        .find(|track| track.name == name)
        .unwrap_or_else(|| panic!("轨 `{name}` 不在规范工程里"))
        .clone()
}

fn preset_device_names(track: &yeban_model::TrackV3) -> Vec<String> {
    track
        .devices
        .iter()
        .map(|device| device.name.clone())
        .filter(|name| name.starts_with("Preset: "))
        .collect()
}

/// 系统清单：6 件工厂预制逐字在场，全部标 `draft: true`。
#[test]
fn list_system_presets_returns_six_factory_presets() {
    let mut dispatcher = dispatcher_with_project();
    let data = call(
        &mut dispatcher,
        "yeban_list_presets",
        serde_json::json!({"kind": "system"}),
    );
    let presets = data["presets"].as_array().expect("presets 数组");
    assert_eq!(presets.len(), 6, "{data}");
    assert_eq!(data["count"], 6, "{data}");
    let ids: Vec<&str> = presets
        .iter()
        .map(|preset| preset["id"].as_str().expect("id 字符串"))
        .collect();
    for expected in [
        "sys:salamander-grand:piano-warm-room",
        "sys:karoryfer-bigcat-cello:cello-warm-hall",
        "sys:freepats-spanish-classical-guitar:guitar-warm-room",
        "sys:ixox-flute:flute-airy-hall",
        "sys:karoryfer-bear-sax:sax-warm-room",
        "sys:karoryfer-big-rusty-drums:drums-punchy-room",
    ] {
        assert!(ids.contains(&expected), "缺工厂预制 `{expected}`: {ids:?}");
    }
    for preset in presets {
        assert_eq!(preset["draft"], true, "工厂预制须标草案待耳验: {preset}");
        assert!(
            preset["instrument"].as_str().is_some_and(|s| !s.is_empty()),
            "工厂预制须带乐器标识: {preset}"
        );
    }
    // `kind=all` 里系统 6 件一分不少（用户件数不钉：它读本机用户库）。
    let all = call(
        &mut dispatcher,
        "yeban_list_presets",
        serde_json::json!({"kind": "all"}),
    );
    let system = all["presets"]
        .as_array()
        .expect("presets 数组")
        .iter()
        .filter(|preset| {
            preset["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("sys:"))
        })
        .count();
    assert_eq!(system, 6, "{all}");
}

/// 未知 kind ⇒ `INVALID_PARAMETER_RANGE`，响亮拒绝。
#[test]
fn list_unknown_kind_is_rejected() {
    let mut dispatcher = dispatcher_with_project();
    call_expect_error(
        &mut dispatcher,
        "yeban_list_presets",
        serde_json::json!({"kind": "bogus"}),
        "INVALID_PARAMETER_RANGE",
    );
}

/// 应用预制：strip+effects 两类只落 2 件 `Preset:` 设备、不碰采样器；
/// 全类应用后采样器绑上大提琴 SFZ；跳过的发送写进提案描述。
#[test]
fn apply_preset_sets_state_and_preset_devices() {
    let mut dispatcher = dispatcher_with_project();
    let bass = track_id(&dispatcher, "Bass");
    let data = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": CELLO_PRESET,
            "categories": ["strip", "effects"],
            "autoMerge": true,
        }),
    );
    assert_eq!(data["autoMerged"], true, "{data}");
    assert!(
        data["proposal"]["title"]
            .as_str()
            .is_some_and(|title| title.contains("应用预制")),
        "提案标题须点名本次应用: {data}"
    );
    let after = track_snapshot(&dispatcher, "Bass");
    assert_eq!(
        after
            .preset
            .as_ref()
            .map(|state| state.preset_id.as_deref()),
        Some(Some(CELLO_PRESET)),
        "轨须记下当前预制"
    );
    assert_eq!(
        after.preset.as_ref().map(|state| state.bypassed),
        Some(false)
    );
    assert!(
        after.preset.as_ref().map(|state| state.previous.is_some()) == Some(true),
        "应用前须拍快照进 `previous`"
    );
    assert_eq!(
        preset_device_names(&after),
        vec!["Preset: Channel Strip", "Preset: Convolution"],
        "strip+effects 两类只落两件设备"
    );
    assert!(after.sampler.is_none(), "没开 instrument 类不得碰采样器");

    let full = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": CELLO_PRESET,
            "autoMerge": true,
        }),
    );
    // 全类应用：发送与宏都动了 —— 目标 Aux 不存在、宏位不够时，跳过的两项须写进提案标题。
    let title = full["proposal"]["title"].as_str().expect("提案标题字符串");
    assert!(
        title.contains("Hall"),
        "跳过的发送（目标 Aux 不存在）须写进提案标题: {title}"
    );
    assert!(
        title.contains("跳过的宏"),
        "跳过的宏（超出当前宏位数）须写进提案标题: {title}"
    );
    // 预制参数全在引擎公告表里：一次应用不得产出 `unrecognisedEffectParam` 误报。
    assert_eq!(
        full["warnings"].as_array().map(Vec::len),
        Some(0),
        "预制参数须全部被引擎认下: {:?}",
        full["warnings"]
    );
    let applied = track_snapshot(&dispatcher, "Bass");
    assert!(
        applied
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(CELLO_SFZ_FRAGMENT)),
        "全类应用须绑大提琴 SFZ: {:?}",
        applied.sampler
    );
    assert_eq!(
        preset_device_names(&applied),
        vec!["Preset: Channel Strip", "Preset: Convolution"],
        "重应用不得重复挂设备"
    );
}

/// 查不到的预制 ⇒ `ENTITY_NOT_FOUND`，不静默回干声。
#[test]
fn apply_unknown_preset_fails_loudly() {
    let mut dispatcher = dispatcher_with_project();
    let bass = track_id(&dispatcher, "Bass");
    call_expect_error(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": "sys:salamander-grand:no-such-preset",
            "autoMerge": true,
        }),
        "ENTITY_NOT_FOUND",
    );
    assert!(
        track_snapshot(&dispatcher, "Bass").preset.is_none(),
        "失败的应用不得留半截状态"
    );
}

/// 旁通：恢复 `previous` 快照（设备清零、乐器不动）；取消旁通：预制设备回来。
#[test]
fn bypass_restores_snapshot_and_reapply_restores_preset() {
    let mut dispatcher = dispatcher_with_project();
    let bass = track_id(&dispatcher, "Bass");
    let _ = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": CELLO_PRESET,
            "autoMerge": true,
        }),
    );
    assert_eq!(
        preset_device_names(&track_snapshot(&dispatcher, "Bass")).len(),
        2,
        "先决：预制设备已落轨"
    );
    let _ = call(
        &mut dispatcher,
        "yeban_preset_bypass",
        serde_json::json!({"trackId": bass, "bypassed": true, "autoMerge": true}),
    );
    let bypassed = track_snapshot(&dispatcher, "Bass");
    assert_eq!(
        preset_device_names(&bypassed).len(),
        0,
        "旁通须拆掉预制设备"
    );
    assert_eq!(
        bypassed.preset.as_ref().map(|state| state.bypassed),
        Some(true)
    );
    assert!(
        bypassed
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(CELLO_SFZ_FRAGMENT)),
        "旁通只旁通处理，不换乐器: {:?}",
        bypassed.sampler
    );
    let _ = call(
        &mut dispatcher,
        "yeban_preset_bypass",
        serde_json::json!({"trackId": bass, "bypassed": false, "autoMerge": true}),
    );
    let restored = track_snapshot(&dispatcher, "Bass");
    assert_eq!(
        preset_device_names(&restored),
        vec!["Preset: Channel Strip", "Preset: Convolution"],
        "取消旁通须按当前预制重新应用"
    );
    assert_eq!(
        restored.preset.as_ref().map(|state| state.bypassed),
        Some(false)
    );
}

/// 自动加载：绑定钢琴采样器即自动应用默认钢琴预制（`autoLoad` 缺省开）。
#[test]
fn auto_load_applies_default_preset_on_sampler_bind() {
    let mut dispatcher = dispatcher_with_project();
    let lead = track_id(&dispatcher, "Lead");
    let data = call(
        &mut dispatcher,
        "yeban_edit_notes",
        serde_json::json!({
            "trackId": lead,
            "ops": [{
                "kind": "setTrackSampler",
                "trackId": lead,
                "sampler": format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}"),
            }],
            "autoMerge": true,
        }),
    );
    assert_eq!(data["autoMerged"], true, "{data}");
    let after = track_snapshot(&dispatcher, "Lead");
    assert!(
        after
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(PIANO_SFZ_FRAGMENT)),
        "采样器须绑上: {:?}",
        after.sampler
    );
    assert_eq!(
        after
            .preset
            .as_ref()
            .map(|state| state.preset_id.as_deref()),
        Some(Some(PIANO_PRESET)),
        "自动加载须记下钢琴预制"
    );
    assert_eq!(
        preset_device_names(&after),
        vec!["Preset: Channel Strip", "Preset: Reverb"],
        "钢琴预制的通道条与混响须落轨"
    );
}

/// 另存：写用户库（文件可读、名与标识一致）；重名响亮拒；`dryRun` 不写盘。
#[test]
fn save_preset_writes_user_library_and_rejects_duplicate_name() {
    let scratch = Scratch::new("save");
    let _home = HomeGuard::redirect(&scratch.dir);
    let mut dispatcher = dispatcher_with_project();
    let lead = track_id(&dispatcher, "Lead");
    // 先绑采样器（自动加载钢琴预制），另存才有快照可写。
    let _ = call(
        &mut dispatcher,
        "yeban_edit_notes",
        serde_json::json!({
            "trackId": lead,
            "ops": [{
                "kind": "setTrackSampler",
                "trackId": lead,
                "sampler": format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}"),
            }],
            "autoMerge": true,
        }),
    );
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "warm-lead"}),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    assert_eq!(saved["instrument"], "salamander-grand", "{saved}");
    assert_eq!(saved["overwritten"], false, "{saved}");
    let id = saved["id"].as_str().expect("id 字符串");
    assert!(id.starts_with("user:"), "{saved}");
    let path = PathBuf::from(saved["path"].as_str().expect("path 字符串"));
    assert_eq!(
        path.parent().expect("父目录"),
        user_library_dir(&scratch.dir),
        "用户预制须写进用户主目录下预制库"
    );
    let bytes = std::fs::read(&path).expect("预制文件须落盘");
    assert!(!bytes.is_empty() && bytes.len() <= 65_536, "预制 ≤ 64 KiB");
    let parsed: Value = serde_json::from_slice(&bytes).expect("预制文件须是合法 JSON");
    assert_eq!(parsed["id"], id, "文件标识须与回执一致");
    assert_eq!(parsed["name"], "warm-lead");
    assert_eq!(parsed["format"], 1);

    // 重名 ⇒ `CONFLICT`，不静默覆盖。
    call_expect_error(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "warm-lead"}),
        "CONFLICT",
    );

    // `dryRun` 只出预览，不写盘（预览在 `data["preview"]` 里，不在顶层）。
    let preview = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "dry-run-only", "dryRun": true}),
    );
    assert_eq!(preview["dryRun"], true, "{preview}");
    let dry_path = PathBuf::from(
        preview["preview"]["path"]
            .as_str()
            .expect("预览须带 path 字符串"),
    );
    assert!(
        !dry_path.exists(),
        "dryRun 不得写盘: {}",
        dry_path.display()
    );
    assert_eq!(
        std::fs::read_dir(user_library_dir(&scratch.dir))
            .expect("读用户库")
            .count(),
        1,
        "用户库里只能有第一份另存"
    );
}

/// 保存收据（设计稿 §5）：缺省带出用户预制（内嵌 1 件）；`includeUserPresets: false`
/// 时内嵌 0 件，接收方打开缺预制会响亮报错（模型层容器读回已在 `preset_track_field`
/// 里钉住，这里只钉收据读数）。
#[test]
fn save_project_receipt_reports_embedded_presets() {
    let scratch = Scratch::new("receipt");
    let _home = HomeGuard::redirect(&scratch.dir);
    let mut dispatcher = dispatcher_with_project();
    let lead = track_id(&dispatcher, "Lead");
    let bass = track_id(&dispatcher, "Bass");
    let _ = call(
        &mut dispatcher,
        "yeban_edit_notes",
        serde_json::json!({
            "trackId": lead,
            "ops": [{
                "kind": "setTrackSampler",
                "trackId": lead,
                "sampler": format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}"),
            }],
            "autoMerge": true,
        }),
    );
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "receipt-v1"}),
    );
    let user_id = saved["id"].as_str().expect("id 字符串").to_owned();
    // 用户预制可被应用（会话内嵌 map 为空时回退到用户库文件）。
    let _ = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": user_id,
            "autoMerge": true,
        }),
    );
    assert_eq!(
        track_snapshot(&dispatcher, "Bass")
            .preset
            .as_ref()
            .map(|state| state.preset_id.clone()),
        Some(Some(user_id.clone())),
        "Bass 须记下用户预制"
    );
    let receipt = call(&mut dispatcher, "yeban_save_project", serde_json::json!({}));
    assert_eq!(receipt["saved"], true, "{receipt}");
    assert_eq!(receipt["embeddedPresets"]["count"], 1, "{receipt}");
    assert!(
        receipt["embeddedPresets"]["bytes"].as_u64().unwrap_or(0) > 0,
        "{receipt}"
    );
    assert_eq!(
        receipt["embeddedPresets"]["includedUserPresets"], true,
        "{receipt}"
    );
    let skipped = call(
        &mut dispatcher,
        "yeban_save_project",
        serde_json::json!({"includeUserPresets": false}),
    );
    assert_eq!(skipped["skipped"], true, "{skipped}");
    assert_eq!(skipped["embeddedPresets"]["count"], 0, "{skipped}");
    assert_eq!(
        skipped["embeddedPresets"]["includedUserPresets"], false,
        "{skipped}"
    );
}
