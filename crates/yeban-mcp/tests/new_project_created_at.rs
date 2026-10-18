//! 判据（任务单 `#215`）：**新建工程写真实创建时刻**，于是导出的 BWF 起源日期
//! 不再是 `1970-01-01`；而"同一个工程文件 ⇒ 同一串导出字节"（`#214`）一位不动。
//!
//! ## 为什么这条判据存在
//!
//! `#214` 把 BWF 的 `OriginationDate` / `OriginationTime` 从**墙钟**改成
//! `project.metadata.created_at_unix_ms`。但全仓**没有**任何生产路径写这个字段
//! （只有迁移器与测试夹具），于是它恒为 `0` ⇒ 两条戳确定地写成 `1970-01-01` /
//! `00:00:00`：可复现，但语义是"未知"。本文件量的是"谁来写这个字段"这一半。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `created_at_unix_ms` | `yeban_open_project {create:true}` 建出的文档 | UNIX 毫秒 |
//! | 创建时刻与判据自己读的墙钟之差 | 同上 | 毫秒（必须很小） |
//! | `bext.origination_date` / `origination_time` | 导出的 WAV 字节里**解析出来的** `bext` | ASCII |
//! | `sha256` | 两个不同输出路径上的 WAV 文件字节 | 十六进制串（必须相等） |
//!
//! ## 三条判据（都能被注入破坏）
//!
//! 1. 新建 ⇒ `created_at_unix_ms > 0` 且 ≈ 现在；`bext` 日期 ≠ `1970-01-01`。
//!    破坏方式：把 `build` 里的 `record_creation_time` 删掉 ⇒ 两条都退回哨兵。
//! 2. 同一工程导出两次（**跨秒**）⇒ `sha256` 相同、`bext` 时间相同。
//!    破坏方式：把墙钟塞回导出路径 ⇒ 第二次的秒个位必变。
//! 3. 旧文件（`created_at == 0`）打开/保存/导出 ⇒ **不补写**：字段仍是 `0`，
//!    导出仍是哨兵 `1970-01-01`（这是**决定**：补写会改旧文件字节）。
//!    破坏方式：在打开或保存路径上顺手 `record_creation_time(now)` ⇒ 字段变非 0。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::project_create::{CreateConfig, build_with_clock};
use yeban_mcp::domain::render_math::bext_stamp;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::container::write_project_container;
use yeban_render::rf64::parse_container;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一次性的临时目录（`Drop` 时删除，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-created-at-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
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
    let dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    match (response.result, response.error) {
        (Some(result), None) => result,
        (None, Some(error)) => panic!("工具路径上不该有 JSON-RPC 错误: {error:?}"),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

/// 当前墙钟（毫秒）—— 判据自己读一次，用来判"创建时刻 ≈ 现在"。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |delta| {
            u64::try_from(delta.as_millis()).unwrap_or(u64::MAX)
        })
}

fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("读产物");
    yeban_model::AssetHash::of_bytes(&bytes).as_str().to_owned()
}

fn render(dispatcher: &mut Dispatcher, auth: &str, out: &Path) -> Value {
    let result = call(
        dispatcher,
        auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "渲染必须成功: {result}");
    result
}

// ---------------------------------------------------------------------------
// 判据 1 + 2：新建工程有真实创建时刻，导出两次逐字节相同且戳不是哨兵
// ---------------------------------------------------------------------------

#[test]
fn a_new_project_exports_a_real_bwf_date_and_stays_reproducible() {
    let scratch = Scratch::new("new");
    let (mut dispatcher, auth) = dispatcher();
    let project_path = scratch.join("fresh.yeban");

    let before = now_ms();
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": project_path.display().to_string(),
            "create": true,
            "title": "Fresh",
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let after = now_ms();

    let project = dispatcher.domain().active_project().expect("活跃工程");
    let created_at = project.metadata.created_at_unix_ms;
    assert!(
        created_at > 0,
        "新建工程必须写真实创建时刻；实得 0 ⇒ BWF 会写 1970-01-01"
    );
    assert!(
        (before..=after).contains(&created_at),
        "创建时刻必须落在本次调用的墙钟窗口里: before={before} created_at={created_at} after={after}"
    );

    // 导出的 WAV 真的落盘两份：**同一个工程**、两个输出路径。
    let first_path = scratch.join("first.wav");
    let second_path = scratch.join("second.wav");
    let first = render(&mut dispatcher, &auth, &first_path);
    // 跨秒：若导出路径上还有墙钟，第二次的 `OriginationTime` 秒个位会变。
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let second = render(&mut dispatcher, &auth, &second_path);

    let first_sha = first["data"]["sha256"].as_str().expect("sha256");
    let second_sha = second["data"]["sha256"].as_str().expect("sha256");
    assert_eq!(first_sha, second_sha, "同一工程导出两次必须逐字节相同");
    assert_eq!(
        first_sha,
        file_sha256(&first_path),
        "响应里的 sha256 必须是磁盘字节的摘要"
    );
    assert_eq!(
        second_sha,
        file_sha256(&second_path),
        "响应里的 sha256 必须是磁盘字节的摘要"
    );

    let parsed = parse_container(&fs::read(&first_path).expect("读 WAV")).expect("解析 WAV");
    let bext = parsed.bext.expect("导出必须写 bext");
    let (expected_date, expected_time) = bext_stamp(created_at);
    assert_eq!(
        (
            bext.origination_date.as_str(),
            bext.origination_time.as_str()
        ),
        (expected_date.as_str(), expected_time.as_str()),
        "BWF 两条戳必须来自工程的创建时刻"
    );
    assert_ne!(
        bext.origination_date, "1970-01-01",
        "新建工程的 BWF 起源日期必须不是哨兵"
    );
    assert_ne!(
        bext.origination_time, "00:00:00",
        "新建工程的 BWF 起源时间必须不是哨兵"
    );

    eprintln!(
        "[#215] create: created_at={created_at} (窗口 {before}..={after}) \
         bwf.date={} bwf.time={} sha256={first_sha} \
         second.sha256={second_sha} 两次导出之间 sleep=1100ms",
        bext.origination_date, bext.origination_time,
    );
}

// ---------------------------------------------------------------------------
// 判据 3：旧工程不被补写（打开 / 保存 / 导出都不会把 0 换成现在）
// ---------------------------------------------------------------------------

#[test]
fn an_old_project_without_a_creation_time_is_not_backfilled() {
    let scratch = Scratch::new("old");
    let (mut dispatcher, auth) = dispatcher();
    let project_path = scratch.join("legacy.yeban");

    // 造一份"旧文件"：可渲染的模板文档，创建时刻 = 0（本单之前所有新工程的样子）。
    let created =
        build_with_clock(&CreateConfig::default(), &project_path, || 0).expect("造旧工程");
    assert_eq!(
        created.project.metadata.created_at_unix_ms, 0,
        "夹具前提：旧文件的创建时刻必须是 0"
    );
    let history = serde_json::to_vec(&yeban_model::commit::CommitGraph::new()).expect("空图谱");
    let bytes = write_project_container(
        &created.project,
        &history,
        &std::collections::BTreeMap::new(),
    )
    .expect("写容器");
    fs::write(&project_path, &bytes).expect("落盘旧文件");
    let before_bytes = fs::read(&project_path).expect("读旧文件");

    // ---- 打开：字段一位不动 ----
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": project_path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    let after_open = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .metadata
        .created_at_unix_ms;
    assert_eq!(after_open, 0, "打开旧工程不得补写创建时刻");

    // ---- 保存：字段一位不动，字节也一位不动 ----
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["status"], "success", "{saved}");
    assert_eq!(
        fs::read(&project_path).expect("读保存后的文件"),
        before_bytes,
        "打开→保存必须是字节稳定的（补写创建时刻会破坏这一点）"
    );
    let after_save = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .metadata
        .created_at_unix_ms;
    assert_eq!(after_save, 0, "保存旧工程不得补写创建时刻");

    // ---- 导出：`0` 的确定语义是"未知" ⇒ 哨兵日期 ----
    let wav = scratch.join("legacy.master.wav");
    let rendered = render(&mut dispatcher, &auth, &wav);
    let parsed = parse_container(&fs::read(&wav).expect("读 WAV")).expect("解析 WAV");
    let bext = parsed.bext.expect("导出必须写 bext");
    assert_eq!(
        (
            bext.origination_date.as_str(),
            bext.origination_time.as_str()
        ),
        ("1970-01-01", "00:00:00"),
        "未记录的创建时刻必须写成确定的哨兵（而不是每次不同的现在）"
    );

    eprintln!(
        "[#215] old: created_at 打开后={after_open} 保存后={after_save} \
         文件字节稳定={} bwf.date={} bwf.time={} sha256={}",
        fs::read(&project_path).expect("读") == before_bytes,
        bext.origination_date,
        bext.origination_time,
        rendered["data"]["sha256"].as_str().expect("sha256"),
    );
}

/// 判据 0（事实复核）：`build_with_clock` 的时钟只被读一次。
///
/// 这条与 `project_create.rs` 的单元判据同一形态，放在端到端文件里是因为它守护的是
/// **工具面那条生产路径**用的同一个函数（`plan_create` → `build` → `build_with_clock`）。
#[test]
fn the_creation_clock_is_read_exactly_once() {
    let calls = std::cell::Cell::new(0_u32);
    let path = std::env::temp_dir().join("yeban-created-at-once.yeban");
    let created = build_with_clock(&CreateConfig::default(), &path, || {
        calls.set(calls.get() + 1);
        1_760_000_000_123
    })
    .expect("建工程");
    assert_eq!(calls.get(), 1, "创建时刻必须只取一次");
    assert_eq!(
        created.project.metadata.created_at_unix_ms,
        1_760_000_000_123
    );
    assert_eq!(
        created.project.metadata.modified_at_unix_ms, 0,
        "本单不写 modified_at（那是'保存时刷新修改时刻'的另一条裁决）"
    );
}
