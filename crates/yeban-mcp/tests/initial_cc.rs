//! ⑨ 用户侧初始 CC（`initialCc`）的工具面判据：经真实 `tools/call` 管线。
//!
//! | 判据 | 量 |
//! | :--- | :--- |
//! | `yeban_load_sfz` 带 `initialCc` | 普查按生效种子算（门控打开 ⇒ 静音 0）、回执回显 `initialCc` |
//! | 不带 `initialCc` | 普查 = 缺省口径（全 0）、回执无 `initialCc` 键（与改前逐字一致） |
//! | 同内容复用 | 第二次装载的 `initialCc` 不生效、回报首次登记的那份（内容寻址既有口径） |
//! | 非法控制器 / 值 | 形状层（非整数 / 负数 / >255）与域层（128..=255）都响亮拒绝 |
//! | 预制 `cc` 段 apply | 与 `load_sfz` 同一条登记路：回执 `initialCcSeeded` 与装载口径一致 |
//! | `dryRun` apply | 不登记（随后装载 `reused == false`） |
//! | save → apply 往返 | 会话里带 `initialCc` 装载过绑定文件 ⇒ 另存的预制带 `cc` 段，新会话 apply 重播同一份种子 |

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

/// 门控夹具：文件初值 `set_cc1=10` 打不开 `locc1=64` 的门；用户值 100 能。
const GATED_SFZ: &str =
    "<control> set_cc1=10\n<region> sample=*silence key=60 locc1=64 hicc1=127\n";

const FLUTE_PATH: &str = "assets/samples/ixox-flute/Ixox Flute.sfz";

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-initialcc-{}-{tag}-{}",
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
/// （与 `preset_tools.rs` 的 `HomeGuard` 同一口径）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

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

fn dispatcher_bare() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
}

fn dispatcher_with_project() -> Dispatcher {
    let mut dispatcher = dispatcher_bare();
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join(format!("yeban-mcp-initialcc-{}.yeban", std::process::id())),
            filled_project(),
            false,
        )
        .expect("注入规范工程");
    dispatcher
}

/// 走真实 `tools/call` 管线，不预设成败（错误断言用它）。
fn call_any(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
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

fn write_gated_sfz(scratch: &Scratch) -> PathBuf {
    let path = scratch.dir.join("gated.sfz");
    std::fs::write(&path, GATED_SFZ).expect("写门控 SFZ 夹具");
    path
}

/// ⭐ 装载带 `initialCc`：种子并入同一张表、普查按生效值算、回执回显；
/// 不带参逐字旧行为；同内容复用以首次登记为准。
#[test]
fn load_sfz_with_initial_cc_seeds_and_reports() {
    let scratch = Scratch::new("load");
    let sfz = write_gated_sfz(&scratch);
    let sfz_arg = sfz.display().to_string();

    // 不带参：普查是缺省口径（文件 set_cc1=10 不进探针 ⇒ 门控仍关、1 条
    // 静音），回执无 initialCc 键。
    let mut plain_session = dispatcher_bare();
    let data = call(
        &mut plain_session,
        "yeban_load_sfz",
        json!({"path": sfz_arg}),
    );
    assert_eq!(data["ccGateCensus"]["silentAtDefaultCc"], 1, "{data}");
    assert!(
        data.get("initialCc").is_none(),
        "不带参回执不得出现 initialCc: {data}"
    );

    // 带 {1: 100}：生效种子打开门控 ⇒ 普查 0 条静音、回执回显。
    let mut session = dispatcher_bare();
    let data = call(
        &mut session,
        "yeban_load_sfz",
        json!({"path": sfz_arg, "initialCc": {"1": 100}}),
    );
    assert_eq!(data["initialCc"], json!({"1": 100}), "{data}");
    assert_eq!(data["ccGateCensus"]["silentAtDefaultCc"], 0, "{data}");
    assert_eq!(data["ccGateCensus"]["byGate"], 0, "{data}");

    // 同内容第二次装载（不同的 initialCc）：内容寻址复用首次登记，本次
    // 取值不生效、回执如实回报登记时的那份。
    let data = call(
        &mut session,
        "yeban_load_sfz",
        json!({"path": sfz_arg, "initialCc": {"1": 30}}),
    );
    assert_eq!(data["reused"], true, "{data}");
    assert_eq!(data["initialCc"], json!({"1": 100}), "{data}");
    assert_eq!(data["ccGateCensus"]["silentAtDefaultCc"], 0, "{data}");
}

/// ⭐ 非法控制器 / 值响亮拒绝：形状层与域层都必须是
/// `INVALID_PARAMETER_RANGE`，且不得钳位成合法值继续装载。
#[test]
fn load_sfz_rejects_illegal_initial_cc() {
    let scratch = Scratch::new("reject");
    let sfz = write_gated_sfz(&scratch);
    let sfz_arg = sfz.display().to_string();
    let mut dispatcher = dispatcher_bare();
    for bad in [
        json!({"128": 1}), // 域层：控制器出 CC 域
        json!({"1": 128}), // 域层：值出 CC 域
        json!({"1": 256}), // 形状层：值装不进 u8
        json!({"256": 1}), // 形状层：控制器装不进 u8
        json!({"1": -1}),  // 形状层：负数
        json!({"1": 1.5}), // 形状层：非整数
        json!({"x": 1}),   // 形状层：键非十进制整数
    ] {
        call_expect_error(
            &mut dispatcher,
            "yeban_load_sfz",
            json!({"path": sfz_arg, "initialCc": bad}),
            "INVALID_PARAMETER_RANGE",
        );
    }
    // 非对象：分发层按注册表声明的类型（`object`）先拒 —— JSON-RPC
    // `-32602`，同样是响亮拒绝、到不了装载（与领域层拒绝的差别只在层）。
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "yeban_load_sfz", "arguments": {"path": sfz_arg, "initialCc": [1, 2]}},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(outcome.error_code(), Some(-32602), "{outcome:?}");
    // 拒绝后会话里不得留下半份登记：合法重装必须是全新登记（reused=false）。
    let data = call(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({"path": sfz_arg, "initialCc": {"1": 100}}),
    );
    assert_eq!(data["reused"], false, "{data}");
}

/// 写一份带 `cc` 段的用户预制到被重定向的用户库，回预制标识。
fn write_user_preset_with_cc(home: &Path, sfz: &Path) -> String {
    let library = user_library_dir(home);
    std::fs::create_dir_all(&library).expect("建用户预制库");
    let ulid = EntityId::new().to_canonical_string();
    let id = format!("user:{ulid}");
    let preset = json!({
        "format": 1,
        "id": id,
        "name": "Initial CC Test",
        "instrument": "test-synth",
        "instrument_sfz": sfz.display().to_string(),
        "scope": "instrument",
        "strip": {},
        "cc": {"1": 100},
    });
    std::fs::write(
        library.join(format!("{ulid}.json")),
        serde_json::to_string_pretty(&preset).expect("序列化预制"),
    )
    .expect("写用户预制");
    id
}

/// ⭐ 预制 `cc` 段 apply：与 `load_sfz` 同一条登记路 —— 提案落地后乐器
/// 登记进会话 registry，回执 `initialCcSeeded` 的生效值与装载口径一致；
/// `dryRun` 不登记。
#[test]
fn apply_preset_with_cc_seeds_the_same_table_as_load_sfz() {
    let scratch = Scratch::new("apply");
    let _home = HomeGuard::redirect(&scratch.dir);
    let sfz = write_gated_sfz(&scratch);
    let preset_id = write_user_preset_with_cc(&scratch.dir, &sfz);
    let lead = {
        let dispatcher = dispatcher_with_project();
        track_id(&dispatcher, "Lead")
    };

    // dryRun：只预览、不登记 —— 随后真装载必须是全新登记。
    let mut dispatcher = dispatcher_with_project();
    let data = call(
        &mut dispatcher,
        "yeban_apply_preset",
        json!({"trackId": lead, "presetId": preset_id, "dryRun": true}),
    );
    assert!(
        data.get("initialCcSeeded").is_none(),
        "dryRun 不得播种: {data}"
    );
    let loaded = call(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({"path": sfz.display().to_string()}),
    );
    assert_eq!(
        loaded["reused"], false,
        "dryRun 后 registry 必须仍为空: {loaded}"
    );

    // 真 apply（autoMerge）：回执带生效种子登记，且与 load 路线同值。
    let mut dispatcher = dispatcher_with_project();
    let data = call(
        &mut dispatcher,
        "yeban_apply_preset",
        json!({"trackId": lead, "presetId": preset_id, "autoMerge": true}),
    );
    let seeded = &data["initialCcSeeded"];
    assert_eq!(seeded["initialCc"], json!({"1": 100}), "{data}");
    assert_eq!(seeded["ccSeedDefaults"], json!({"1": 100}), "{data}");
    // 同一会话随后装载同一文件（不带参）：命中内容寻址复用 ⇒ registry 里
    // 的正是 apply 登记的那份（初始 CC 与普查都随登记走）。
    let loaded = call(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({"path": sfz.display().to_string()}),
    );
    assert_eq!(loaded["reused"], true, "{loaded}");
    assert_eq!(loaded["initialCc"], json!({"1": 100}), "{loaded}");
    assert_eq!(loaded["ccGateCensus"]["silentAtDefaultCc"], 0, "{loaded}");
}

/// ⭐ 存 / 用一致：会话里带 `initialCc` 装载过轨绑定文件 ⇒ 另存预制自动
/// 带上 `cc` 段；新会话 apply 该预制 ⇒ 重播同一份种子。
#[test]
fn save_preset_captures_initial_cc_and_apply_replays_it() {
    let scratch = Scratch::new("roundtrip");
    let _home = HomeGuard::redirect(&scratch.dir);

    let mut dispatcher = dispatcher_with_project();
    let lead = track_id(&dispatcher, "Lead");
    // ① 带 initialCc 装载长笛（工厂乐器，另存要求 slug 可识别）。
    let loaded = call(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({"path": FLUTE_PATH, "initialCc": {"7": 90}}),
    );
    assert_eq!(loaded["initialCc"], json!({"7": 90}), "{loaded}");
    // ② 把 Lead 绑到同一份 SFZ。
    call(
        &mut dispatcher,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": FLUTE_PATH}],
            "autoMerge": true,
        }),
    );
    // ③ 另存：预制文件必须带 cc 段（捕获自会话登记）。
    let saved = call(
        &mut dispatcher,
        "yeban_save_preset",
        json!({"trackId": lead, "name": "CC Capture"}),
    );
    let preset_id = saved["id"].as_str().expect("另存回执带 id").to_owned();
    let library = user_library_dir(&scratch.dir);
    let files: Vec<PathBuf> = std::fs::read_dir(&library)
        .expect("读用户库")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    assert_eq!(files.len(), 1, "用户库里恰一份预制: {files:?}");
    let text = std::fs::read_to_string(&files[0]).expect("读预制文件");
    let parsed: Value = serde_json::from_str(&text).expect("预制是 JSON");
    assert_eq!(parsed["cc"], json!({"7": 90}), "另存须捕获初始 CC: {text}");

    // ④ 新会话 apply：重播同一份种子。
    let mut fresh = dispatcher_with_project();
    let lead = track_id(&fresh, "Lead");
    let data = call(
        &mut fresh,
        "yeban_apply_preset",
        json!({"trackId": lead, "presetId": preset_id, "autoMerge": true}),
    );
    assert_eq!(
        data["initialCcSeeded"]["initialCc"],
        json!({"7": 90}),
        "{data}"
    );
    assert_eq!(
        data["initialCcSeeded"]["ccSeedDefaults"]["7"],
        json!(90),
        "{data}"
    );
}
