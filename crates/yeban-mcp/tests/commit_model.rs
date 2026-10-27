//! ⭐ `ADR-0032`（缺口 `G17`）：**每条成功回执自述 `commitModel`**。
//!
//! 缺口的形状：同一次作曲里 `edit_notes` / `propose_section` / `set_macro` 走
//! **提案 + 合并**，而 `yeban_edit_automation` 走**直接提交** —— 调用方必须**记住**
//! 哪个工具还要再去 merge。本文件钉的就是"不必记"这件事：回执自己说。
//!
//! ## 判据地图（`ADR-0032` 末尾登记的四条）
//!
//! | # | 判据 | 可被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | 六类真实调用的 `data.commitModel` 逐字等于 D1 表；`proposal` 那条同时 `pendingMerge: true` | 注入点被挪走 / 某个 `apply_*` 手写覆盖 |
//! | 2 | `direct` / `none` 两类回执里⛔ 不得出现 `mergeTool`（把"要记的规矩"变成断言）；`list_proposals` 的**说明性**同名字段是实测到的例外，另行钉住 | 键集合扩张时把 merge 语义漏进非提案类工具 |
//! | 3 | 取值**恰好四条**（加第五条必须故意改这里，与 `tools::TOOL_COUNT` 同一口径） | 悄悄加态 ⇒ 老调用方读到没见过的值 |
//! | 4 | `dryRun` 预览**不带**这个键（D2：⛔ 不在 `dry_run_result` 里另算一套） | 预览与真实回执各说一套 |
//! | 5 | 五个已声明 `outputSchema` 的工具把 `commitModel` 同时写进 `properties` 与 `required`（D3 的契约义务） | 声明面落后于实现面 = schema 不再是"从真实响应抽的" |
//!
//! 全部落在 `std::env::temp_dir()` 下的唯一子目录，[`Scratch`] 的 `Drop` 负责删除 —— ⛔ 不污染仓库。

use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::COMMIT_MODEL_VALUES;
use yeban_mcp::output_schema;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::COMMIT_MODEL_KEY;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-commit-model-{tag}-{}",
            std::process::id()
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

fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线，返回成功响应的 `data`。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "commit-model",
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
    let result = response.result.expect("必须有 result");
    assert_eq!(result["status"], "success", "{name} 工具层失败: {result}");
    result["data"].clone()
}

/// 建一个带**一条普通音轨**的新工程（`seed.trackCount` 给 `edit_automation` 落点）。
fn open_seeded(dispatcher: &mut Dispatcher, auth: &str, scratch: &Scratch) -> String {
    let path = scratch.dir.join("commit-model.yebanproject");
    let data = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "commitModel 判据工程",
            "bpm": 120.0,
            "seed": {"trackCount": 1, "clipName": "Commit Model", "notes": []},
        }),
    );
    let project = &data["project"];
    let master = project["master_bus_track_id"]
        .as_str()
        .expect("主总线身份")
        .to_owned();
    project["tracks"]
        .as_object()
        .expect("tracks 必须是对象")
        .keys()
        .find(|id| **id != master)
        .cloned()
        .expect("种子里就有一条非主总线音轨")
}

/// 一次提案的最小实参（`materialSource: "theory"` ⇒ 空素材池也能走）。
fn section(label: &str, extra: Value) -> Value {
    let mut arguments = json!({
        "sectionName": label,
        "stylePreset": "pop",
        "materialSource": "theory",
        "bars": 2,
    });
    if let (Some(map), Some(extra)) = (arguments.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
    }
    arguments
}

/// ⭐ 判据 1：六类真实调用各自的 `commitModel` 逐字等于 `ADR-0032` **D1** 表。
#[test]
fn commit_model_is_self_described_per_category() {
    let scratch = Scratch::new("categories");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let track = open_seeded(&mut dispatcher, &auth, &scratch);

    // ① 提案（缺省）⇒ `proposal`，且"必须再去 merge"这句话由 `pendingMerge` 同向印证。
    let proposal = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        section("Verse", json!({})),
    );
    assert_eq!(proposal[COMMIT_MODEL_KEY], json!("proposal"), "{proposal}");
    assert_eq!(proposal["pendingMerge"], json!(true), "{proposal}");

    // ② 提案 + `autoMerge` ⇒ `proposalAutoMerged`（同一次调用里已经合并完了）。
    let auto_merged = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        section("Chorus", json!({"autoMerge": true})),
    );
    assert_eq!(
        auto_merged[COMMIT_MODEL_KEY],
        json!("proposalAutoMerged"),
        "{auto_merged}"
    );
    assert_eq!(auto_merged["pendingMerge"], json!(false), "{auto_merged}");

    // ③ 只读查询 ⇒ `none`。
    let query = call(&mut dispatcher, &auth, "yeban_query_project", json!({}));
    assert_eq!(query[COMMIT_MODEL_KEY], json!("none"), "{query}");

    // ④ `merge` 本身就是"合并这一步" ⇒ `direct`（⛔ 不是回到 `proposal` 语义）。
    let proposal_id = proposal["proposal"]["proposalId"]
        .as_str()
        .expect("提案必须带身份")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id}),
    );
    assert_eq!(merged["merged"], json!(true), "{merged}");
    assert_eq!(merged[COMMIT_MODEL_KEY], json!("direct"), "{merged}");

    // ⑤ 直接提交的自动化编辑 ⇒ `direct`。
    let automation = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_automation",
        json!({
            "trackId": track,
            "lane": "TrackVolume",
            "point": {"tick": 0, "value": 0.5, "curve": "Linear"},
        }),
    );
    assert_eq!(
        automation[COMMIT_MODEL_KEY],
        json!("direct"),
        "直接提交类工具必须自述 direct: {automation}"
    );

    // ⑥ 撤销移动的是工程头，没有"下一步去合并" ⇒ `direct`。
    let undo = call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo[COMMIT_MODEL_KEY], json!("direct"), "{undo}");

    println!(
        "### commitModel 六类读数\n  proposal={}\n  proposalAutoMerged={}\n  query={}\n  merge={}\n  automation={}\n  undo={}",
        proposal[COMMIT_MODEL_KEY],
        auto_merged[COMMIT_MODEL_KEY],
        query[COMMIT_MODEL_KEY],
        merged[COMMIT_MODEL_KEY],
        automation[COMMIT_MODEL_KEY],
        undo[COMMIT_MODEL_KEY],
    );
}

/// ⭐ 判据 2：`direct` / `none` 两类回执⛔ 不得出现 `mergeTool` ——
/// "这次不用再去 merge"从记忆变成断言，正是本票的交付。
///
/// ⚠ 一条**实测到的例外**（先读交付，再下结论）：`yeban_list_proposals` 的清单里
/// 顶层就带一个 `"mergeTool": "yeban_merge_proposal"`（`domain/mod.rs:3188`，`#176` 的
/// 既有契约）—— 它说的是"**清单里那些提案**该用哪个工具去合"，⛔ 不是"本次调用属于
/// 提案模型"。两者同名不同义，因此本判据**不**把它算作违规，另在文件末尾把它的
/// `commitModel == "none"` 钉住：两把钥匙各说各的、不互相冒充。
#[test]
fn non_proposal_receipts_do_not_promise_a_merge_step() {
    let scratch = Scratch::new("no-merge-tool");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let track = open_seeded(&mut dispatcher, &auth, &scratch);

    let for_each: [(&str, Value); 3] = [
        (
            "yeban_query_project",
            call(&mut dispatcher, &auth, "yeban_query_project", json!({})),
        ),
        (
            "yeban_query_engine_state",
            call(
                &mut dispatcher,
                &auth,
                "yeban_query_engine_state",
                json!({"trackId": track}),
            ),
        ),
        (
            "yeban_edit_automation",
            call(
                &mut dispatcher,
                &auth,
                "yeban_edit_automation",
                json!({
                    "trackId": track,
                    "lane": "TrackVolume",
                    "point": {"tick": 0, "value": 0.4, "curve": "Linear"},
                }),
            ),
        ),
    ];
    for (name, data) in for_each {
        assert!(
            data.get(COMMIT_MODEL_KEY).is_some(),
            "{name} 的成功回执必须自述提交模型: {data}"
        );
        assert!(
            data.get("mergeTool").is_none(),
            "`{name}` 不属于提案家族 ⇒ 不得教调用方去 merge: {data}"
        );
    }

    // 上面那条例外本身：清单是 `none`（它自己一条提交都不产生），但**照旧**带说明性的
    // `mergeTool` ⇒ 这条既有契约不因本票改变（`ADR-0032` D1 的"⛔ 不改任何既有键的含义"）。
    call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        section("Bridge", json!({})),
    );
    let list = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    assert_eq!(list[COMMIT_MODEL_KEY], json!("none"), "{list}");
    assert_eq!(
        list["openCount"],
        json!(1),
        "清单里现在有一条 open 提案: {list}"
    );
    assert_eq!(
        list["mergeTool"],
        json!("yeban_merge_proposal"),
        "清单的既有说明字段保持不变: {list}"
    );
}

/// ⭐ 判据 3：取值**恰好四条**，且 `outputSchema` 里声明的枚举与实现同源（D3）。
#[test]
fn commit_model_values_are_exactly_four_and_schemas_agree() {
    assert_eq!(
        COMMIT_MODEL_VALUES,
        ["proposal", "proposalAutoMerged", "direct", "none"],
        "增删一条取值必须**故意**改本判据（与 `tools::TOOL_COUNT` 同一口径）"
    );
    let distinct = COMMIT_MODEL_VALUES
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert_eq!(distinct, 4, "四条取值必须互不相同");

    for name in output_schema::COVERED_TOOLS {
        let schema = output_schema::for_tool(name).expect("五个主力工具都必须声明 schema");
        assert_eq!(
            schema["properties"][COMMIT_MODEL_KEY]["type"],
            json!("string"),
            "`{name}` 必须声明 commitModel 的类型: {schema}"
        );
        assert_eq!(
            schema["properties"][COMMIT_MODEL_KEY]["enum"],
            json!(COMMIT_MODEL_VALUES),
            "`{name}` 声明的取值集合必须与实现同源（⛔ 两份清单）: {schema}"
        );
        assert!(
            schema["required"]
                .as_array()
                .expect("required 必须是数组")
                .iter()
                .any(|value| *value == json!(COMMIT_MODEL_KEY)),
            "`{name}` 把每条回执都存在的键写进 `required` 是**契约义务**（D3）: {schema}"
        );
    }
    println!(
        "### commitModel 声明面: 覆盖 {} 个工具 / 取值 {COMMIT_MODEL_VALUES:?}",
        output_schema::COVERED_TOOLS.len()
    );
}

/// ⭐ 判据 4：`dryRun` 预览⛔ 不带这个键（D2 —— 预览已有 `sideEffect` /
/// `wouldChangeState` 说同一件事，另算一套就是两个权威面）。
#[test]
fn dry_run_preview_does_not_claim_a_commit_model() {
    let scratch = Scratch::new("dry-run");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_seeded(&mut dispatcher, &auth, &scratch);

    let preview = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        section("Intro", json!({"dryRun": true})),
    );
    assert_eq!(preview["dryRun"], json!(true), "{preview}");
    assert_eq!(preview["stateUnchanged"], json!(true), "{preview}");
    assert!(
        preview
            .as_object()
            .expect("预览信封必须是对象")
            .get(COMMIT_MODEL_KEY)
            .is_none(),
        "`dryRun` 压根不进 `apply_inner` ⇒ 预览不得冒充回执的自述: {preview}"
    );
}
