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
//! | 创建钩子 | 带 `seed.sampler` 新建后 `preset_id` | 钢琴预制标识 |
//! | 创建钩子 | 不带 sampler 新建 | 首轨无预制 |
//! | 创建钩子 | 带 sampler 新建后旁通 | 设备拆掉、采样器保留 |
//! | 另存 | 用户库文件 / 重名 | 落盘 / `CONFLICT` |
//! | 保存收据 | 内嵌预制数（带出 / 不带出） | 1 / 0 |

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::{demo_project, filled_project};

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

/// 清空指定轨的设备链并绑上工厂采样器（模拟"只有 `Preset:` 设备"的工厂 SFZ 轨，
/// 用来对照不含手工设备时预制字节不变）。
fn with_clean_sampler_track(
    mut project: yeban_model::YebanProjectV1,
    track: &str,
) -> yeban_model::YebanProjectV1 {
    let target = project
        .tracks
        .values_mut()
        .find(|candidate| candidate.name == track)
        .unwrap_or_else(|| panic!("轨 `{track}` 不在演示工程里"));
    target.devices.clear();
    target.sampler = Some(yeban_model::SamplerBinding {
        note_octave_override: None,
        path: format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}"),
    });
    project
}

/// 给工程里名为 `track` 的轨手工插一台 EQ（参数 `low_gain = 6.0`）。
///
/// 这是票面工作流的起点：**手工插入**的设备名不带 `Preset: ` 前缀，
/// 旧 `snapshot_track` 会直接跳过它。
fn with_eq_device(
    mut project: yeban_model::YebanProjectV1,
    track: &str,
) -> yeban_model::YebanProjectV1 {
    let target = project
        .tracks
        .values_mut()
        .find(|candidate| candidate.name == track)
        .unwrap_or_else(|| panic!("轨 `{track}` 不在演示工程里"));
    target.devices.push(yeban_model::DeviceDefinition {
        id: EntityId::new(),
        name: "EQ".to_owned(),
        kind: yeban_model::DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![yeban_model::ParameterValue {
            name: "low_gain".to_owned(),
            value: 6.0,
            unit: None,
        }],
        latency_samples: 0,
        plugin: None,
    });
    project
}

/// 把 `$HOME` 指到临时目录（用户预制库随之搬家），析构时恢复。
struct HomeGuard {
    old_home: Option<String>,
    old_userprofile: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HomeGuard {
    fn redirect(dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let old_home = std::env::var("HOME").ok();
        let old_userprofile = std::env::var("USERPROFILE").ok();
        // SAFETY: 持 ENV_LOCK；本测试进程内没有其它线程在此时改 `$HOME` / `$USERPROFILE`。
        unsafe {
            std::env::set_var("HOME", dir);
            std::env::set_var("USERPROFILE", dir);
        }
        Self {
            old_home,
            old_userprofile,
            _lock: lock,
        }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // SAFETY: 与 `redirect` 同一条理由；恢复的是进入时读到的原值。
        unsafe {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
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
        .open_in_memory_with_assets(
            std::env::temp_dir().join(format!("yeban-mcp-preset-{}.yeban", std::process::id())),
            filled_project(),
            false,
            yeban_model::samples::filled_project_assets(),
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

/// 无活跃工程的调度器（`create:true` 用；已有活跃工程时再建新工程会 `CONFLICT`）。
fn dispatcher_empty() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
}

/// 经真实 `tools/call` 管线新建工程（`create:true`），回 `data`。
fn create_project(dispatcher: &mut Dispatcher, path: &Path, seed: Value) -> Value {
    call(
        dispatcher,
        "yeban_open_project",
        serde_json::json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Hook",
            "seed": seed,
        }),
    )
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
    // T-FIX-660 判据：系统鼓组预制的样本文件在 `assets/samples` 里缺失，
    // 列表必须把「要样本包 / 缺多少」说清楚，⛔ 不能和可发声预制长得一样。
    let drums = presets
        .iter()
        .find(|preset| preset["id"] == "sys:karoryfer-big-rusty-drums:drums-punchy-room")
        .expect("系统鼓组预制必须在清单里");
    assert_eq!(
        drums["requiresSamplePack"], true,
        "缺样本的鼓组预制必须标 requiresSamplePack: {drums}"
    );
    assert!(
        drums["sampleStatus"]["missingSampleCount"]
            .as_u64()
            .is_some_and(|missing| missing > 0),
        "鼓组预制必须报出缺失样本数: {drums}"
    );
    assert_eq!(
        drums["sampleStatus"]["parseError"],
        Value::Null,
        "鼓组 SFZ 必须可解析，缺的是音频文件: {drums}"
    );
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

/// T-FIX-660：套用样本缺失的系统鼓组预制必须**成功但响亮** ——
/// `warnings[]` 里带 `presetSamplesMissing`，报明缺多少；⛔ 不静默成功。
#[test]
fn apply_preset_with_missing_samples_warns_loudly() {
    let mut dispatcher = dispatcher_with_project();
    let bass = track_id(&dispatcher, "Bass");
    let data = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({
            "trackId": bass,
            "presetId": "sys:karoryfer-big-rusty-drums:drums-punchy-room",
            "categories": ["instrument"],
            "autoMerge": true,
        }),
    );
    assert_eq!(data["autoMerged"], true, "套用本身必须成功: {data}");
    let sampler_warnings = data["samplerWarnings"]
        .as_array()
        .expect("samplerWarnings 是数组");
    // 换绑时同一数组还会有逐样本 `MissingSample`（`sample` 非 null）；
    // 这里只取“预制整体”那一条聚合告警（`sample=null` 且 detail 点名 preset）。
    let missing: Vec<&Value> = sampler_warnings
        .iter()
        .filter(|warning| warning["sample"].is_null())
        .filter(|warning| {
            warning["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("preset `sys:karoryfer-big-rusty-drums"))
        })
        .collect();
    assert_eq!(
        missing.len(),
        1,
        "缺样本的预制必须在套用回执里恰好点名一次: {sampler_warnings:?}"
    );
    let detail = missing[0]["detail"].as_str().expect("detail 字符串");
    assert!(
        detail.contains("missingSampleCount"),
        "告警必须报出缺失样本数: {detail}"
    );
    assert!(
        detail.contains("declaredSampleCount"),
        "告警必须报出声明样本数: {detail}"
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
    // T-FIX-266：warnings 里除了 effect-param 还会带 `samplerBinding` 缺样本预检
    // （样本按 ADR-0024 外取，未安装时须点名）。原断言把两类混为一谈，看到任何
    // 告警就红；这里只断言"预制参数全被引擎认下"这件事。
    let warnings = full["warnings"].as_array().expect("warnings 是数组");
    let effect_param: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["kind"].as_str() == Some("unrecognisedEffectParam"))
        .collect();
    assert!(
        effect_param.is_empty(),
        "预制参数须全部被引擎认下: {:?}",
        effect_param
    );
    // samplerBinding 必须按 (binding, sample) 去重并聚合：一条告警带 count 与
    // 最多 5 条 examples，响应有界，且 instrument 内的 ../Samples 不得被判越界。
    let sampler_bindings: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["kind"].as_str() == Some("samplerBinding"))
        .collect();
    for warning in &sampler_bindings {
        let count = warning["count"].as_u64().expect("聚合告警必须有 count");
        assert!(count >= 1, "count 至少为 1: {:?}", warning);
        assert!(
            warning["examples"].as_array().is_some_and(|a| a.len() <= 5),
            "examples 最多 5 条: {:?}",
            warning
        );
        assert_eq!(
            warning["blocking"].as_bool(),
            Some(false),
            "缺样本只警告、不阻断: {:?}",
            warning
        );
    }
    let warnings_bytes = serde_json::to_string(warnings)
        .expect("warnings 可序列化")
        .len();
    assert!(
        warnings_bytes < 4096,
        "聚合后 warnings 须远小于逐条平铺的量级（< 4KB），实际 {warnings_bytes} B；告警 {sampler_bindings:?}"
    );
    assert!(
        !serde_json::to_string(&sampler_bindings)
            .expect("可序列化")
            .contains("escapes the SFZ root"),
        "instrument 内的 ../Samples 路径不得再被判 SampleEscape: {:?}",
        sampler_bindings
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

/// 旁通：恢复 `previous` 快照（设备清零、**乐器一并回** —— T-FIX-409）；取消旁通：预制设备与乐器回来。
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
        bypassed.sampler.is_none(),
        "旁通必须连乐器一起回：应用前 Bass 没有采样器 ⇒ 旁通后应为 None，实际: {:?}",
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
    assert!(
        restored
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(CELLO_SFZ_FRAGMENT)),
        "取消旁通必须按预制重绑乐器: {:?}",
        restored.sampler
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

/// 创建钩子：带 `seed.sampler` 新建 ⇒ 首轨绑采样器并自动加载默认钢琴预制。
#[test]
fn create_with_seed_sampler_auto_loads_default_preset() {
    let scratch = Scratch::new("create-hook-a");
    let mut dispatcher = dispatcher_empty();
    let path = scratch.dir.join("with-sampler.yeban");
    let data = create_project(
        &mut dispatcher,
        &path,
        serde_json::json!({"sampler": format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}")}),
    );
    assert_eq!(data["created"], true, "{data}");
    assert!(path.exists(), "工程容器必须真的落盘: {}", path.display());
    let first = track_snapshot(&dispatcher, "Track 1");
    assert!(
        first
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(PIANO_SFZ_FRAGMENT)),
        "采样器须绑上: {:?}",
        first.sampler
    );
    assert_eq!(
        first
            .preset
            .as_ref()
            .map(|state| state.preset_id.as_deref()),
        Some(Some(PIANO_PRESET)),
        "自动加载须记下钢琴预制"
    );
    assert_eq!(
        preset_device_names(&first),
        vec!["Preset: Channel Strip", "Preset: Reverb"],
        "钢琴预制的通道条与混响须落轨"
    );
}

/// 创建钩子：不带 `seed.sampler` 新建 ⇒ 首轨无采样器、无预制（旧行为逐字节不变）。
#[test]
fn create_without_seed_sampler_applies_no_preset() {
    let scratch = Scratch::new("create-hook-b");
    let mut dispatcher = dispatcher_empty();
    let path = scratch.dir.join("no-sampler.yeban");
    let data = create_project(&mut dispatcher, &path, serde_json::json!({}));
    assert_eq!(data["created"], true, "{data}");
    let first = track_snapshot(&dispatcher, "Track 1");
    assert_eq!(first.sampler, None, "不给 sampler 不得绑采样器");
    assert_eq!(first.preset, None, "不给 sampler 不得加载预制");
}

/// 创建钩子：带 `seed.sampler` 新建后旁通 ⇒ 预制设备拆掉、采样器保留、`bypassed` 为 true。
/// （“旁通时不加载”指旁通后预制处理不生效，即本条断言；工具面没有设 `autoLoad=false` 的开关。）
#[test]
fn create_with_seed_sampler_bypass_removes_preset_devices() {
    let scratch = Scratch::new("create-hook-c");
    let mut dispatcher = dispatcher_empty();
    let path = scratch.dir.join("bypass.yeban");
    let _ = create_project(
        &mut dispatcher,
        &path,
        serde_json::json!({"sampler": format!("assets/samples/salamander-grand/{PIANO_SFZ_FRAGMENT}")}),
    );
    let first_id = track_snapshot(&dispatcher, "Track 1")
        .id
        .to_canonical_string();
    let _ = call(
        &mut dispatcher,
        "yeban_preset_bypass",
        serde_json::json!({"trackId": first_id, "bypassed": true, "autoMerge": true}),
    );
    let after = track_snapshot(&dispatcher, "Track 1");
    assert_eq!(preset_device_names(&after).len(), 0, "旁通须拆掉预制设备");
    assert_eq!(
        after.preset.as_ref().map(|state| state.bypassed),
        Some(true)
    );
    assert!(
        after
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(PIANO_SFZ_FRAGMENT)),
        "旁通只旁通处理，不换乐器: {:?}",
        after.sampler
    );
}

/// 判据（T-FIX-409）：旁通**连乐器一起回**——应用前没绑采样器的轨，旁通后
/// 采样器必须回到 `None`（不是仍响预制钢琴）；取消旁通再按预制重绑。
#[test]
fn bypass_restores_the_sampler_binding_too() {
    let scratch = Scratch::new("bypass-sampler");
    let _home = HomeGuard::redirect(&scratch.dir);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-preset-bypass-{}.yeban",
                std::process::id()
            )),
            demo_project(),
            false,
        )
        .expect("注入演示工程");

    let track = track_id(&dispatcher, "鼓");
    let before = track_snapshot(&dispatcher, "鼓");
    assert!(
        before.sampler.is_none(),
        "前提：演示工程的「鼓」轨没有采样器: {:?}",
        before.sampler
    );
    let before_devices = preset_device_names(&before).len();

    let applied = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({"trackId": track, "presetId": PIANO_PRESET, "autoMerge": true}),
    );
    assert_eq!(applied["autoMerged"], true, "{applied}");
    let after_apply = track_snapshot(&dispatcher, "鼓");
    assert!(
        after_apply
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(PIANO_SFZ_FRAGMENT)),
        "应用含乐器的预制后必须绑上钢琴: {:?}",
        after_apply.sampler
    );

    let bypassed = call(
        &mut dispatcher,
        "yeban_preset_bypass",
        serde_json::json!({"trackId": track, "bypassed": true, "autoMerge": true}),
    );
    assert_eq!(bypassed["autoMerged"], true, "{bypassed}");
    let after_bypass = track_snapshot(&dispatcher, "鼓");
    assert_eq!(
        after_bypass.sampler, before.sampler,
        "旁通后采样器必须回到应用前（None）: {:?}",
        after_bypass.sampler
    );
    assert_eq!(
        preset_device_names(&after_bypass).len(),
        before_devices,
        "旁通后预制设备数必须回到应用前"
    );

    let un_bypassed = call(
        &mut dispatcher,
        "yeban_preset_bypass",
        serde_json::json!({"trackId": track, "bypassed": false, "autoMerge": true}),
    );
    assert_eq!(un_bypassed["autoMerged"], true, "{un_bypassed}");
    let after_resume = track_snapshot(&dispatcher, "鼓");
    assert!(
        after_resume
            .sampler
            .as_ref()
            .is_some_and(|sampler| sampler.path.contains(PIANO_SFZ_FRAGMENT)),
        "取消旁通后必须按预制重绑钢琴: {:?}",
        after_resume.sampler
    );
}

/// 判据（T-FIX-390）：`scope:"track"` 不要求采样器 —— 内置合成器 MIDI 轨与音频轨都能存
/// 整轨快照，预制里乐器记 `internal`、`instrumentSfz` 为空（应用时不重绑采样器）；
/// `scope:"instrument"` 仍要求已知乐器，但错误必须说清是"内置合成器"还是"音频轨"。
#[test]
fn save_track_preset_needs_no_sampler_and_instrument_scope_names_the_real_reason() {
    let scratch = Scratch::new("track-scope");
    let _home = HomeGuard::redirect(&scratch.dir);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-preset-scope-{}.yeban",
                std::process::id()
            )),
            demo_project(),
            false,
        )
        .expect("注入演示工程（无采样器轨）");

    let midi = track_id(&dispatcher, "鼓");
    let audio = track_id(&dispatcher, "贝斯");

    // ① 内置合成器 MIDI 轨：整轨快照可存，乐器记 `internal`、sfz 为空。
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": midi, "name": "whole-midi", "scope": "track"}),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    assert_eq!(saved["scope"], "track", "{saved}");
    let parsed: Value = serde_json::from_slice(
        &std::fs::read(saved["path"].as_str().expect("path")).expect("读库"),
    )
    .expect("合法 JSON");
    assert_eq!(parsed["instrument"], "internal", "{parsed}");
    assert_eq!(parsed["instrument_sfz"], "", "{parsed}");

    // ② 音频轨：整轨快照同样可存（不再报"未绑定采样器"）。
    let saved_audio = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": audio, "name": "whole-audio", "scope": "track"}),
    );
    assert_eq!(saved_audio["saved"], true, "{saved_audio}");

    // ③ `scope:"instrument"`：两档错误各自点名真实原因。
    let midi_result = call_any(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": midi, "name": "inst-midi", "scope": "instrument"}),
    );
    assert_eq!(midi_result["status"], "error", "{midi_result}");
    assert_eq!(
        midi_result["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{midi_result}"
    );
    let midi_error = midi_result["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        midi_error.contains("内置合成器"),
        "MIDI 无采样器须点名内置合成器，实际: {midi_error}"
    );
    assert!(
        midi_error.contains("scope") && midi_error.contains("track"),
        "须给出 `scope:\"track\"` 的出路，实际: {midi_error}"
    );
    let audio_result = call_any(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": audio, "name": "inst-audio", "scope": "instrument"}),
    );
    assert_eq!(audio_result["status"], "error", "{audio_result}");
    assert_eq!(
        audio_result["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{audio_result}"
    );
    let audio_error = audio_result["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        audio_error.contains("音频轨"),
        "音频轨须点名「音频轨」，⛔ 不许说成「无法确定适用乐器」，实际: {audio_error}"
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

/// 判据（T-FIX-414，缺口 2 / 裁决 A）：手工插入的 EQ `low_gain = 6.0` 存成整轨
/// 预制后，应用到**另一条轨**必须把参数带过去（确定性身份、先拆旧链）。
#[test]
fn eq_device_flows_through_a_track_preset_to_another_track() {
    let scratch = Scratch::new("eq-flow");
    let _home = HomeGuard::redirect(&scratch.dir);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-preset-eq-{}.yeban", std::process::id())),
            with_eq_device(demo_project(), "主音"),
            false,
        )
        .expect("注入带手工 EQ 的演示工程");

    let lead = track_id(&dispatcher, "主音");
    let pad = track_id(&dispatcher, "铺底");
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "eq-plus-six", "scope": "track"}),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    let id = saved["id"].as_str().expect("id").to_owned();

    let applied = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({"trackId": pad, "presetId": id, "autoMerge": true}),
    );
    assert_eq!(applied["autoMerged"], true, "{applied}");
    let after = track_snapshot(&dispatcher, "铺底");
    let eq = after
        .devices
        .iter()
        .find(|device| device.name == "EQ")
        .unwrap_or_else(|| panic!("EQ 没有被带过去: {:?}", after.devices));
    let low_gain = eq
        .params
        .iter()
        .find(|param| param.name == "low_gain")
        .map(|param| param.value)
        .expect("EQ 须带 low_gain");
    assert_eq!(low_gain, 6.0, "手工 EQ 的 low_gain 须逐值带过去");

    // 再应用一次同一份预制 ⇒ 链条先拆再建，不得叠成两台 EQ。
    call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({"trackId": pad, "presetId": id, "autoMerge": true}),
    );
    let twice = track_snapshot(&dispatcher, "铺底");
    assert_eq!(
        twice
            .devices
            .iter()
            .filter(|device| device.name == "EQ")
            .count(),
        1,
        "重复应用要先拆旧捕获链（否则叠设备）: {:?}",
        twice.devices
    );
}

/// 判据（T-FIX-414，缺口 1 / 探针 1 收编）：`internal` 整轨预制存下后
/// **列得出、应用得了**。旧 `validate()` 要求 `instrument_sfz` 非空 ⇒
/// 保存腿不校验、读取腿校验，文件一写出来就「损坏」。
#[test]
fn internal_track_preset_is_listed_and_applicable() {
    let scratch = Scratch::new("internal-list-apply");
    let _home = HomeGuard::redirect(&scratch.dir);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-preset-internal-{}.yeban",
                std::process::id()
            )),
            demo_project(),
            false,
        )
        .expect("注入演示工程");

    let midi = track_id(&dispatcher, "鼓");
    let pad = track_id(&dispatcher, "铺底");
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": midi, "name": "internal-whole", "scope": "track"}),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    let id = saved["id"].as_str().expect("id").to_owned();

    let listed = call(
        &mut dispatcher,
        "yeban_list_presets",
        serde_json::json!({"kind": "user"}),
    );
    assert_eq!(
        listed["skipped"],
        serde_json::json!([]),
        "不得有损坏条目: {listed}"
    );
    assert!(
        listed["presets"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["id"] == id.as_str())),
        "internal 整轨预制必须列得出来: {listed}"
    );

    let applied = call(
        &mut dispatcher,
        "yeban_apply_preset",
        serde_json::json!({"trackId": pad, "presetId": id, "autoMerge": true}),
    );
    assert_eq!(applied["autoMerged"], true, "应用腿必须成功: {applied}");
    assert_eq!(
        track_snapshot(&dispatcher, "铺底")
            .preset
            .as_ref()
            .map(|state| state.preset_id.clone()),
        Some(Some(id)),
        "应用后轨须记下该预制"
    );
}

/// 判据（T-FIX-414，缺口 2 的字节面 / 探针 2 收编）：工厂 SFZ 轨的整轨预制
/// 不含手工设备 ⇒ `devices` 整段省略，序列化字节与旧格式逐字一致（不升 format）。
#[test]
fn factory_track_preset_stays_byte_identical_without_captured_devices() {
    let scratch = Scratch::new("factory-bytes");
    let _home = HomeGuard::redirect(&scratch.dir);
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!(
                "yeban-mcp-preset-bytes-{}.yeban",
                std::process::id()
            )),
            with_clean_sampler_track(demo_project(), "主音"),
            false,
        )
        .expect("注入只有工厂采样器、无手工设备的工程");
    let lead = track_id(&dispatcher, "主音");
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        serde_json::json!({"trackId": lead, "name": "factory-whole", "scope": "track"}),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    let text = std::fs::read_to_string(saved["path"].as_str().expect("path")).expect("读库");
    let parsed: Value = serde_json::from_str(&text).expect("合法 JSON");
    assert!(
        parsed.get("devices").is_none(),
        "没有手工设备的预制不得写出 devices 段（旧预制字节不变）: {text}"
    );
    assert_eq!(parsed["format"], 1, "不得升 format: {text}");
}
