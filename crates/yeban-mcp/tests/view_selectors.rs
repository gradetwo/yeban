//! `SELECTORS` ↔ `schemas/project.schema.json` 的**双向**对账。
//!
//! ## 为什么必须有这条判据（第 44 条：手抄清单必须对账）
//!
//! `domain/view.rs` 的 [`SELECTORS`] 是 `yeban_query_project` 那个 `select` 参数的
//! **白名单**。它把 schema 的顶层字段名**手抄**了一遍 —— 于是"schema 加了一个字段"
//! 与"白名单知道那个字段"是**两件独立的事**：
//!
//! - `view.rs` 里**有**运行期检查（`SELECTORS.contains(&name)` ⇒ 未知选择器报
//!   `unknownSelector` 并回传 `availableSelectors`）；
//! - ⛔ 但那条检查**只挡调用方**，⛔ **不挡 schema 演进** —— 谁往
//!   `project.schema.json` 加一个顶层字段，白名单**不会**跟着动，
//!   而且**没有任何东西会提醒**。那正是本仓反复抓到的病
//!   （`op_reachability` 的单向登记表、`tools.rs` 漏 3 个 kind 的描述）。
//!
//! ⭐ 所以本判据**现读** schema 文件（⛔ 不抄第二份清单），并做**双向**断言。
//!
//! ## 判什么、为什么这样判
//!
//! | 方向 | 断言 | 挡住的病 |
//! | :--- | :--- | :--- |
//! | ① `SELECTORS` 的**顶层**项 ⊆ schema 顶层键 | 白名单里有个 **schema 没有**的字段名 ⇒ 调用方按它查必然查不到（假选择器） |
//! | ② schema 顶层键 ⊆ `SELECTORS` 顶层项 ∪ [`EXEMPT_SCHEMA_KEYS`] | schema 加了字段而白名单没跟 ⇒ **静默漏字段**（本判据的核心价值） |
//!
//! ⚠ **豁免清单是显式的、逐条带理由的**（与 `PENDING_CONTRACT_OPS` /
//! `KNOWN_UNREACHABLE` 同型）：⛔ 不许为了让判据变绿而把断言放宽成**单向** ——
//! 那正是这条判据要防的病。今天该清单**为空**（两侧精确相等）。
//!
//! ## 抽不到东西必须响亮失败（第 41 条）
//!
//! 一个**失灵的解析器**（路径写错、schema 改成没有 `properties` 的形状）会让
//! "集合相等"变成"空 == 空" ⇒ 判据**永远绿**。因此 [`schema_top_level_keys`]
//! **自己**就断言"抽到的键不为空"，⛔ 不把这件事留给上层。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use yeban_mcp::domain::view::SELECTORS;

/// `schemas/project.schema.json` 的路径（`<crate>/../../schemas/...`）。
///
/// 与 `tests/contract.rs` 的 `schema_path()` 同一条相对路径规则。
fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schemas")
        .join("project.schema.json")
}

/// 读并解析 schema。
fn schema() -> Value {
    let text = std::fs::read_to_string(schema_path()).expect("读 schemas/project.schema.json");
    serde_json::from_str(&text).expect("project.schema.json 必须是合法 JSON")
}

/// schema 的 `properties` **顶层键**集合。
///
/// ⚠ **抽不到东西就响亮失败**（第 41 条）：路径错了、或 schema 换了个没有
/// `properties` 的形状 ⇒ 这里直接炸，⛔ 不会静默返回空集合让上层"空 == 空"过掉。
fn schema_top_level_keys() -> BTreeSet<String> {
    let keys: BTreeSet<String> = schema()["properties"]
        .as_object()
        .expect(
            "schemas/project.schema.json 的根必须有 `properties` 对象 \
             (抽不到 ⇒ 本判据会永远绿, 所以这里必须响亮失败)",
        )
        .keys()
        .cloned()
        .collect();
    assert!(
        !keys.is_empty(),
        "schema 的 `properties` 抽出了 0 个键 —— 解析路径已失灵, ⛔ 不许静默通过"
    );
    keys
}

/// [`SELECTORS`] 里**顶层**的那一部分（⛔ 去掉 `tracks.id` 那样的一层下钻）。
///
/// 判据：下钻项**必须**是 `<顶层键>.<叶子>`，且 `<顶层键>` 在顶层集合里 ——
/// 否则一个拼错的下钻项（`trakcs.id`）会**绕过**顶层对账。
fn selector_roots() -> BTreeSet<String> {
    let all: BTreeSet<&str> = SELECTORS.iter().copied().collect();
    assert_eq!(all.len(), SELECTORS.len(), "`SELECTORS` 不得有重复项");
    assert!(
        !all.is_empty(),
        "`SELECTORS` 抽出了 0 项 —— ⛔ 不许静默通过"
    );
    let roots: BTreeSet<String> = SELECTORS
        .iter()
        .filter(|selector| !selector.contains('.'))
        .map(|selector| (*selector).to_owned())
        .collect();
    assert!(
        !roots.is_empty(),
        "`SELECTORS` 里一个顶层项都没有 —— 形状变了, ⛔ 不许静默通过"
    );
    roots
}

/// [`SELECTORS`] 里**下钻**的那一部分（形如 `tracks.id`）。
fn selector_drills() -> Vec<&'static str> {
    SELECTORS
        .iter()
        .copied()
        .filter(|selector| selector.contains('.'))
        .collect()
}

/// ⭐ **显式豁免**：schema 里**允许**不被 `SELECTORS` 覆盖的顶层键（每条带理由）。
///
/// 与 `PENDING_CONTRACT_OPS` / `KNOWN_UNREACHABLE` 同型：**欠账要显式**。
/// ⛔ 不许把方向②的断言删掉来"解决"红 —— 要么把字段加进 `SELECTORS`，要么在这里
/// 登记并写明理由。
///
/// ⚠ **不写自述式「今天为空」**（那种写法会在**首次加豁免**时烂掉 —— 见台账 R456 口径）：
/// 现读条数**见常量本身**，且下方判据会打印实测两侧集合；
/// 「两侧相等」这一事实由判据断言，⛔ 不由这句注释担保。
const EXEMPT_SCHEMA_KEYS: [(&str, &str); 0] = [
    // 形如：
    // (
    //     "some_top_level_key",
    //     "为什么**故意**不暴露给 `select`：(逐字理由)",
    // ),
];

/// 方向 ① ＋ ②：`SELECTORS` 顶层项 与 schema 顶层键 **双向**相等（允许显式豁免）。
#[test]
fn the_selector_whitelist_matches_the_project_schema_both_ways() {
    let schema_keys = schema_top_level_keys();
    let roots = selector_roots();
    let exempt: BTreeSet<&str> = EXEMPT_SCHEMA_KEYS.iter().map(|(key, _)| *key).collect();

    // 豁免项自己必须"在 schema 里、且不在 SELECTORS 里" —— 否则它是**死登记**：
    // 一条无效的豁免会**悄悄**放宽方向②（与 `op_reachability` 的
    // `KNOWN_UNREACHABLE` 被塞进一个已可达变体是同一种病）。
    for (key, reason) in EXEMPT_SCHEMA_KEYS {
        assert!(
            !reason.trim().is_empty(),
            "豁免项 `{key}` 必须带**非空**理由"
        );
        assert!(
            schema_keys.contains(key),
            "豁免项 `{key}` 根本不在 schema 顶层键里 ⇒ 它是死登记, 请删掉"
        );
        assert!(
            !roots.contains(key),
            "豁免项 `{key}` **已经在** `SELECTORS` 里 ⇒ 两侧已相等, 该豁免是多余的, 请删掉"
        );
    }

    // ① 白名单**不许**出现 schema 没有的顶层字段名（假选择器）。
    let ghosts: BTreeSet<_> = roots.difference(&schema_keys).cloned().collect();
    assert!(
        ghosts.is_empty(),
        "`SELECTORS` 里有 schema **没有**的顶层字段名: {ghosts:?} \
         (调用方按它查永远查不到 ⇒ 假选择器)"
    );

    // ② schema 的每个顶层键**必须**在白名单里（或显式豁免）—— 这条挡"schema 演进"。
    let missing: BTreeSet<_> = schema_keys
        .difference(&roots)
        .filter(|key| !exempt.contains(key.as_str()))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "schema 里这些顶层键**没有被 `select` 白名单覆盖**: {missing:?} \
         ⇒ 要么把它们加进 `SELECTORS`, 要么在 `EXEMPT_SCHEMA_KEYS` 里逐条写明理由 \
         (⛔ 不许把本判据放宽成单向)"
    );

    // 逐条读数（红的时候上面两条已经说明问题；这里把"没有漂移"这件事也钉住）。
    assert_eq!(
        roots.len(),
        schema_keys.len() - EXEMPT_SCHEMA_KEYS.len(),
        "双向相等 ⇒ 顶层项数必须 = schema 顶层键数 − 豁免数 \
         (现读: SELECTORS 顶层 {} 个 / schema 顶层 {} 个 / 豁免 {} 条)",
        roots.len(),
        schema_keys.len(),
        EXEMPT_SCHEMA_KEYS.len()
    );
}

/// 下钻项必须是 `<顶层键>.<叶子>`，且根在顶层集合里。
///
/// ⚠ 为什么单列：方向①②只对**顶层**对账 ⇒ 一个拼错的根（`trakcs.id`）
/// 会被"顶层之外"的过滤器**放过去**。这条把那个洞堵上。
#[test]
fn every_drill_down_selector_has_a_known_root() {
    let roots = selector_roots();
    let drills = selector_drills();
    assert!(
        !drills.is_empty(),
        "本仓当前**有**下钻选择器 (如 `tracks.id`) ⇒ 抽到 0 个说明提取逻辑失灵, \
         ⛔ 不许静默通过"
    );
    for drill in drills {
        let (root, leaf) = drill
            .split_once('.')
            .unwrap_or_else(|| panic!("下钻项 `{drill}` 必须形如 `<顶层键>.<叶子>`"));
        assert!(
            roots.contains(root),
            "下钻项 `{drill}` 的根 `{root}` 不在顶层白名单里 ⇒ 它绕过顶层对账"
        );
        assert!(!leaf.is_empty(), "下钻项 `{drill}` 的叶子不能为空");
    }
}

/// `SELECTORS` 是**白名单语义**：`select` 传未知名字必须被拒（运行期那条检查的钉子）。
///
/// ⚠ 这条不读 schema，它钉的是"白名单**真的**在被执行"（否则上面几条只证明
/// 一份清单与一个文件相等，⛔ 证明不了调用方会碰到它）。
#[test]
fn the_whitelist_is_actually_enforced_at_runtime() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("domain")
            .join("view.rs"),
    )
    .expect("读 src/domain/view.rs");
    assert!(
        source.contains("SELECTORS.contains("),
        "`src/domain/view.rs` 必须真的用 `SELECTORS.contains(…)` 做运行期白名单检查"
    );
}
