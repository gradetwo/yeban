//! ⭐ 补丁 `P1` 的判据：`yeban_load_sfz` 对**相对路径**走与渲染侧同一套四档解析。
//!
//! ## 缺陷（逐字，来自《出埃及》工程的真实撞墙）
//!
//! ```text
//! YEBAN_SAMPLE_LIBRARY=<库根>
//! yeban_load_sfz {"path": "assets/samples/hungarian-zither/Programs/hungarian_zither.sfz"}
//! → IO_ERROR  entity not found
//! ```
//!
//! 而**同一条路径**写进 `setTrackSampler` 后 `yeban_render_master` 一切正常。根因：
//! `domain/sfz.rs::parse_load_arguments` 只做 `PathBuf::from(path)`，随后 `load_with`
//! 直接 `fs::metadata` / `fs::read` —— **不经过** `render::resolve_sampler_path`
//! （四档：绝对 → 工程目录 → 采样库根 → 仓库根 → cwd）。于是"哪条路径算数"在工具面
//! 有**两套**答案：装载要绝对，渲染要相对；而 `docs/SFZ_SAMPLE_ACQUISITION.md` 教的
//! 正是把字节放进采样库、工程里存 `assets/samples/<乐器>/…` 这种相对形态。
//!
//! ## 判据读数
//!
//! | # | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | 1 | 库根形态的相对路径（`assets/samples/<乐器>/<文件>.sfz`）修前 ⇒ `IO_ERROR` / `sfzReadFailed` | 复现缺陷（本文件先于修复落地并实测红） |
//! | 2 | 同一路径修后 ⇒ `success`、`regions > 0`、`data.path` 落在**库根** | 修好了 |
//! | 3 | 正对照：**绝对路径**的 `sha256` / `regions` / `path` 与修前逐字相同 | 严格扩张，不改既有行为 |
//! | 4 | 四档都不中 ⇒ 仍 `IO_ERROR` / `sfzReadFailed`，且 `data.path` 是**原样**字符串 | 不发明路径、错误码不变 |
//! | 5 | **无活跃工程**时同一相对路径照样能解析（少掉"工程目录"一档） | `project_dir = None` 不是死路 |
//! | 6 | 工程目录**优先于**库根：两处同名 SFZ（库根那份是坏文本）⇒ 取工程目录那份 | 顺序与渲染侧一致 |
//!
//! ## 反向腿（注入 ⇒ 哪条红）
//!
//! - **注入甲**：`parse_load_arguments` 里改回 `PathBuf::from(path)` ⇒ 判据 1/2/5/6 红；
//! - **注入乙**：解析顺序改成"库根优先" ⇒ 判据 6 红；
//! - **注入丙**：解析不出时**发明**一个路径（而不是原样返回）⇒ 判据 4 红（`data.path`
//!   不再逐字等于入参）；
//! - **注入丁**：绝对路径也走四档 ⇒ 判据 3 红（`data.path` 会被换成库根候选的拼写）。
//!
//! ## 临时目录 / 环境变量纪律
//!
//! 与 `sampler_path_roots.rs` 同一条：本文件是**独立测试进程**，`YEBAN_SAMPLE_LIBRARY`
//! 是进程级状态 ⇒ 全部判据串行持 [`ENV_LOCK`]，用 RAII 守卫恢复原值；夹具落在
//! `std::env::temp_dir()/<唯一子目录>`，[`Scratch`] 的 `Drop` 删除 —— ⛔ 不污染仓库。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

/// 进程级环境状态的串行锁（`YEBAN_SAMPLE_LIBRARY`）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 与 app 的 `yeban_app::samples::SAMPLE_LIBRARY_ENV` 逐字相同。
const SAMPLE_LIBRARY_ENV: &str = "YEBAN_SAMPLE_LIBRARY";

/// 一份**真的能解析出区域**的 SFZ 文本：一个区域、内建静音 ⇒ 不依赖任何 WAV 字节，
/// 于是本文件的判据只考"路径解析"，不考"样本够不够"。
const ONE_REGION: &str = "<region> sample=*silence key=60 pitch_keycenter=60\n";

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-p1-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 在 `dir` 下按相对路径写一份 SFZ（自动建父目录），返回绝对路径。
    fn put(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("建夹具父目录");
        }
        std::fs::write(&path, body).expect("写 SFZ 夹具");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 设置 `YEBAN_SAMPLE_LIBRARY` 的 RAII 守卫（`Drop` 恢复原值）。
struct LibraryEnv {
    previous: Option<OsString>,
}

impl LibraryEnv {
    /// ⚠ 调用方必须持有 [`ENV_LOCK`]。
    fn set(dir: &Path) -> Self {
        let previous = std::env::var_os(SAMPLE_LIBRARY_ENV);
        // SAFETY: 调用方持有 ENV_LOCK；本测试进程内没有其它线程读这个变量。
        unsafe {
            std::env::set_var(SAMPLE_LIBRARY_ENV, dir);
        }
        Self { previous }
    }
}

impl Drop for LibraryEnv {
    fn drop(&mut self) {
        // SAFETY: 与 `set` 同一条理由；恢复的是进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(SAMPLE_LIBRARY_ENV, value),
                None => std::env::remove_var(SAMPLE_LIBRARY_ENV),
            }
        }
    }
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `result`（要求**没有** JSON-RPC 层错误）。
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
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 一个新分发器：**有**活跃工程（工程文件落在 `project_path`，不必真的存在）。
fn dispatcher_with_project_at(project_path: &Path) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.to_path_buf(), filled_project(), false)
        .expect("注入规范工程");
    dispatcher
}

/// 一个新分发器：**没有**活跃工程。
fn dispatcher_without_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
}

/// 装载失败时的 `data`（断言 `reason` 与逐字 `path`）。
fn load_failure(result: &Value) -> &Value {
    assert_eq!(
        result["status"], "error",
        "本判据要求装载**失败**（成功就说明路径被解析到了）: {result}"
    );
    assert_eq!(result["error"]["code"], "IO_ERROR", "{result}");
    &result["error"]["data"]
}

/// 装载成功时的 `data`。
fn load_success(result: &Value) -> &Value {
    assert_eq!(result["status"], "success", "装载必须成功: {result}");
    &result["data"]
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// ⭐ P1：库根形态的相对路径修后能装载 —— 与渲染侧说同一句话。
///
/// 这个用例同时是判据 1 与 2 的"同一条路径、两种行为"：修前它红在
/// `IO_ERROR / sfzReadFailed`，修后绿在 `success`。
#[test]
fn load_sfz_resolves_library_relative_path() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let scratch = Scratch::new("library");
    let library = scratch.join("library");
    std::fs::create_dir_all(&library).expect("建库根");
    let absolute = scratch.put("library/karoryfer-fixture/one_region.sfz", ONE_REGION);
    let _env = LibraryEnv::set(&library);

    // 工程文件旁边**没有**这份 SFZ ⇒ 只有库根那一档能命中。
    let project_path = scratch.join("projects/demo.yeban");
    std::fs::create_dir_all(project_path.parent().expect("父目录")).expect("建工程目录");
    let mut dispatcher = dispatcher_with_project_at(&project_path);

    // 库内存的是 `<库根>/<乐器>/<文件>`，工程里存的是带 `assets/samples/` 前缀的形态。
    let relative = "assets/samples/karoryfer-fixture/one_region.sfz";
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": relative }),
    );
    let data = load_success(&loaded);
    assert!(
        data["regions"].as_u64().unwrap_or(0) > 0,
        "必须真的解析出区域: {data}"
    );
    // 路径按**组件**比较（`#281`：Windows 上 `/` 与 `\` 是同一条路径的两种拼写）。
    assert_eq!(
        PathBuf::from(data["path"].as_str().expect("path 字符串")),
        absolute,
        "修后 `path` 必须落在库根那份文件上: {data}"
    );
    assert_eq!(data["reused"], false, "首次装载不得标复用: {data}");
}

/// ⭐ P1 的**正对照**：绝对路径的行为逐字节不变。
///
/// 修前修后都应拿到同一个 `sha256`，且 `data.path` 逐字等于入参 —— 这条钉住
/// "严格扩张"：解析只在**相对**路径上发生。
#[test]
fn an_absolute_load_path_is_untouched_by_the_resolver() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let scratch = Scratch::new("absolute");
    let library = scratch.join("library");
    std::fs::create_dir_all(&library).expect("建库根");
    let absolute = scratch.put("library/fixture/one_region.sfz", ONE_REGION);
    let _env = LibraryEnv::set(&library);

    let project_path = scratch.join("projects/demo.yeban");
    std::fs::create_dir_all(project_path.parent().expect("父目录")).expect("建工程目录");
    let mut dispatcher = dispatcher_with_project_at(&project_path);

    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": absolute.display().to_string() }),
    );
    let data = load_success(&loaded);
    assert_eq!(
        PathBuf::from(data["path"].as_str().expect("path 字符串")),
        absolute,
        "绝对路径必须原样回显: {data}"
    );
    assert!(
        data["regions"].as_u64().unwrap_or(0) > 0,
        "绝对路径照常解析区域: {data}"
    );
}

/// ⭐ P1：四档都不中 ⇒ 错误码与 `data.path` 形状不变（⛔ 不发明路径）。
#[test]
fn an_unresolvable_load_path_fails_with_the_raw_path() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let scratch = Scratch::new("missing");
    let library = scratch.join("library");
    std::fs::create_dir_all(&library).expect("建库根");
    let _env = LibraryEnv::set(&library);

    let project_path = scratch.join("projects/demo.yeban");
    std::fs::create_dir_all(project_path.parent().expect("父目录")).expect("建工程目录");
    let mut dispatcher = dispatcher_with_project_at(&project_path);

    let relative = "assets/samples/definitely-absent/nope.sfz";
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": relative }),
    );
    let data = load_failure(&loaded);
    assert_eq!(
        data["reason"], "sfzReadFailed",
        "错误原因词表不变: {data}"
    );
    assert_eq!(
        data["path"], relative,
        "解析不出时必须**原样**回显入参（⛔ 不发明路径）: {data}"
    );
}

/// ⭐ P1：**没有活跃工程**时，除"工程目录"以外的三档照样生效。
#[test]
fn a_library_relative_path_loads_without_an_active_project() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let scratch = Scratch::new("no-project");
    let library = scratch.join("library");
    std::fs::create_dir_all(&library).expect("建库根");
    let absolute = scratch.put("library/fixture/one_region.sfz", ONE_REGION);
    let _env = LibraryEnv::set(&library);

    let mut dispatcher = dispatcher_without_project();
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": "assets/samples/fixture/one_region.sfz" }),
    );
    let data = load_success(&loaded);
    assert_eq!(
        PathBuf::from(data["path"].as_str().expect("path 字符串")),
        absolute,
        "`project_dir = None` 不得变成死路: {data}"
    );
}

/// ⭐ P1：**工程目录优先于库根**（与 `render::resolve_sampler_path` 的候选次序一致）。
///
/// 同一个绝对路径 `raw`，在**工程目录**与**库根**各有一份：库根那份是坏文本
/// （`<region` 不闭合 ⇒ 解析器拒绝），工程目录那份是一个合法区域。顺序反了就会取到
/// 库根那份 ⇒ 报 `sfzParseFailed`；顺序对了 ⇒ `success` 且 `data.path` 逐字是工程目录那份。
///
/// 探针刻意用**名字不可能在别处存在**的乐器目录（`p1-order-fixture`），并且**在
/// `assets/samples/` 前缀下**造夹具：不这么做的话，"仓库根"那一档（`CARGO_MANIFEST_DIR/../..`）
/// 会先命中仓库里真实的 `assets/samples/<真乐器>/`，判据测到的就不是顺序了。
#[test]
fn the_project_directory_wins_over_the_library_root() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let scratch = Scratch::new("order");
    // 采样库根：`<库根>/p1-order-fixture/one_region.sfz`（坏文件）
    let library = scratch.join("library");
    let library_copy = scratch.put("library/p1-order-fixture/one_region.sfz", "<region\n");
    let _env = LibraryEnv::set(&library);

    // 工程目录：`<工程目录>/assets/samples/p1-order-fixture/one_region.sfz`（好文件）
    let projects = scratch.join("projects");
    std::fs::create_dir_all(&projects).expect("建工程目录");
    let project_copy =
        scratch.put("projects/assets/samples/p1-order-fixture/one_region.sfz", ONE_REGION);
    assert_ne!(project_copy, library_copy, "两份必须真的不是同一个文件");

    let mut dispatcher = dispatcher_with_project_at(&projects.join("demo.yeban"));
    let loaded = call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": "assets/samples/p1-order-fixture/one_region.sfz" }),
    );
    let data = load_success(&loaded);
    assert_eq!(
        PathBuf::from(data["path"].as_str().expect("path 字符串")),
        project_copy,
        "工程目录必须优先（否则拿到的会是库根那份坏文件）: {data}"
    );
    assert!(
        data["regions"].as_u64().unwrap_or(0) > 0,
        "取到工程目录那份 ⇒ 区域数 > 0: {data}"
    );
}
