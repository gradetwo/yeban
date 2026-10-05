//! **`dryRun`（先问后做）** 在 UI 控制面上的词表与语义 —— ADR-0001 **D48**。
//!
//! 裁决原文（D48）：`yeban-ui-mcp` 的控制方法增加 `dryRun`（只回报"将要发生什么"、
//! 不改状态），并与领域侧 `yeban_*` 工具的 `dryRun` 语义**对齐**（同一个词必须同一个意思）。
//!
//! ## "对齐"在这份代码里是怎么落地的（不是靠人眼比对）
//!
//! | 对齐项 | 机制 | 出处（**领域侧**） |
//! | :--- | :--- | :--- |
//! | 参数名 `dryRun` | **同一个常量**：[`DRY_RUN_PARAM`] 是 `yeban_mcp::tools::DRY_RUN_PARAM` 的再导出 | `crates/yeban-mcp/src/tools.rs:37` |
//! | 响应旗标 `dryRun` | **同一个常量**：[`DRY_RUN_FLAG`] 是 `yeban_mcp::dispatch::DRY_RUN_FLAG` 的再导出 | `crates/yeban-mcp/src/dispatch.rs:58` |
//! | 默认值 `false` | 判据 `dry_run_defaults_to_false_exactly_like_the_domain` 用**真** `yeban_mcp::tools::ToolCall` 逐形状对账 | `crates/yeban-mcp/src/tools.rs:771`（`is_dry_run`） |
//! | `wouldChangeState` | 判据 `payload_carries_every_domain_field_name_verbatim` 拿**真管线**产出的 dryRun 载荷取键名 | `crates/yeban-mcp/src/dispatch.rs:402` |
//! | `stateUnchanged` | 同上 | `crates/yeban-mcp/src/dispatch.rs:414` |
//! | `requiredScope` / `arguments` / `preview` | 同上 | `crates/yeban-mcp/src/dispatch.rs:406,410,415` |
//!
//! **再导出而不是照抄**是刻意的：照抄一份 `"dryRun"` 字面量也能让今天的判据变绿，
//! 但它在"领域侧改名"的那一天不会变红 —— 而"同一个词同一个意思"恰恰要求那一天变红。
//!
//! ## 两处**有意**的不同（写在这里，免得读者以为是漏掉的）
//!
//! 1. **身份字段**：领域侧给的是 `tool` / `specId`（一个工具一个规范 ID），UI 控制面给的是
//!    `method`（线格式方法名）/ `specIds`（一条方法可以同时实现多条规范，例如
//!    `ui/screenshot` 挂了 4 个 ID）。身份字段**属于各自端点**，dryRun 的**语义**字段才是共享词表。
//! 2. **不加 `sideEffect`**：领域侧的 `SideEffect` 是**闭合三值**
//!    （`read-only` / `project-state` / `disk`，`crates/yeban-mcp/src/tools.rs:250-276`），
//!    里面**没有"界面状态"这一档**（与 ADR-0001 的 D29 勘误同因：六级 scope 也没有
//!    "界面状态写"）。给 `ui/switch_main_view` 硬塞一个 `project-state` 是**谎报**，
//!    所以本线只报共享的 `wouldChangeState` 布尔（它的语义与领域侧
//!    `side_effect.is_side_effecting()` 逐字相同），不借用那个闭合枚举。
//!
//! ## 为什么 `dryRun` 只给**会改状态**的方法（14 条里的 7 条）
//!
//! D48 的原文是"给**会改状态**的 `ui/*` 方法加 `dryRun`"。只读方法
//! （`ui/tree` / `ui/node` / `ui/property` / `ui/dynamic_regions` / `ui/screenshot` /
//! `ui/coverage` / `ui/methods`）**没有副作用可短路**，给它们加 `dryRun` 只会制造
//! 一种病理形状：`ui/screenshot` + `dryRun` 让调用方以为"我拿到了一帧但没渲染"。
//! 因此本线的规则是**可判定的**：`mutating == true` ⟺ 声明了 `dryRun`
//! （判据 `every_mutating_method_declares_dry_run_and_no_read_only_method_does`）。
//!
//! 只读方法收到 `dryRun` 时**响亮拒绝**（`-32602`，未知参数一律拒绝的既有口径，
//! 见 `crate::methods` 的 M9 注释），**不静默忽略** —— 静默忽略会让调用方以为
//! "我的 dryRun 生效了"，那是最坏的一种成功。

use serde_json::{Map, Value};

use crate::methods::MethodSpec;

/// `dryRun` 的**参数名** —— 与领域侧**同一个常量**（不是照抄的字面量）。
pub use yeban_mcp::tools::DRY_RUN_PARAM;

/// `dryRun` 的**响应旗标名** —— 与领域侧**同一个常量**（不是照抄的字面量）。
pub use yeban_mcp::dispatch::DRY_RUN_FLAG;

/// "调用前后状态逐字段相同"的旗标（领域侧 `dispatch.rs:414` 的键名，逐字）。
pub const STATE_UNCHANGED: &str = "stateUnchanged";
/// "这条方法**在原则上**会改状态"的旗标（领域侧 `dispatch.rs:402` 的键名，逐字）。
pub const WOULD_CHANGE_STATE: &str = "wouldChangeState";
/// 网络层要求的 scope（领域侧 `dispatch.rs:406` 的键名，逐字）。
pub const REQUIRED_SCOPE: &str = "requiredScope";
/// 归一化实参（领域侧 `dispatch.rs:410` 的键名，逐字）。
pub const ARGUMENTS: &str = "arguments";
/// "将要发生什么"的结构化预览（领域侧 `dispatch.rs:415` 的键名，逐字）。
pub const PREVIEW: &str = "preview";
/// 线格式方法名（UI 控制面的身份字段；见模块头"两处有意的不同"）。
pub const METHOD: &str = "method";
/// 本方法实现的规范 ID 清单（UI 控制面的身份字段）。
pub const SPEC_IDS: &str = "specIds";
/// 预览里的动作名（`preview.operation`）。
pub const OPERATION: &str = "operation";
/// 预览里执行面报的**只读影响**（`preview.effect`；执行面给不出时为 `null`）。
pub const EFFECT: &str = "effect";

/// `dryRun` 是否为真。
///
/// 默认 `false` —— **与领域侧逐字一致**（`yeban_mcp::tools::ToolCall::is_dry_run`：
/// `arguments.get("dryRun").and_then(Value::as_bool).unwrap_or(false)`，
/// `crates/yeban-mcp/src/tools.rs:771`）。判据
/// `dry_run_defaults_to_false_exactly_like_the_domain` 对四种形状逐一对账。
///
/// 参数校验（`crate::methods::validate_params`）已经把 `dryRun` 的类型钉成 `boolean`，
/// 因此这里的 `and_then(Value::as_bool)` 是**第二道**（防御性，不改变语义）。
#[must_use]
pub fn requested(params: &Map<String, Value>) -> bool {
    params
        .get(DRY_RUN_PARAM)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// `dryRun` **自身**不进实参快照（与领域侧 `ToolCall::domain_arguments()` 同口径：
/// `dryRun` 不是领域/执行面语义）。`_` 前缀的 MCP 保留键同样不进（它们不是本方法的参数）。
#[must_use]
pub fn normalized_arguments(params: &Map<String, Value>) -> Map<String, Value> {
    let mut map = params.clone();
    map.remove(DRY_RUN_PARAM);
    map.retain(|key, _| !key.starts_with(crate::methods::RESERVED_PARAM_PREFIX));
    map
}

/// `dryRun=true` 的响应载荷。
///
/// 形状是**平的**（不是 `{"status":…,"data":…}` 的 `ToolResponse`）：UI 控制面的线上契约
/// 是 JSON-RPC 2.0 的**自由结果对象**，把它包成领域契约实例属于类型错误
/// （见 `crate::samples` 的 `methods_document` 注释与 `docs/ledger/mcp-core-notes.md` §2 M12/M13）。
///
/// 键序固定（`serde_json` 未开 `preserve_order` ⇒ `Map` 是 `BTreeMap`，键按字母序），
/// 因此同一份调用两次的响应逐字节相同。
#[must_use]
pub fn payload(
    spec: &MethodSpec,
    operation: &str,
    arguments: Map<String, Value>,
    effect: Option<Value>,
) -> Value {
    debug_assert!(
        spec.mutating,
        "`{}` 是只读方法, 不该走到 dryRun 载荷（注册表与执行分支漂移）",
        spec.name
    );
    let mut preview = Map::new();
    preview.insert(OPERATION.to_owned(), Value::from(operation));
    preview.insert(EFFECT.to_owned(), effect.unwrap_or(Value::Null));

    let mut root = Map::new();
    root.insert(DRY_RUN_FLAG.to_owned(), Value::from(true));
    root.insert(STATE_UNCHANGED.to_owned(), Value::from(true));
    root.insert(WOULD_CHANGE_STATE.to_owned(), Value::from(spec.mutating));
    root.insert(METHOD.to_owned(), Value::from(spec.name));
    root.insert(REQUIRED_SCOPE.to_owned(), Value::from(spec.scope.as_str()));
    root.insert(
        SPEC_IDS.to_owned(),
        Value::Array(spec.spec_ids.iter().map(|id| Value::from(*id)).collect()),
    );
    root.insert(ARGUMENTS.to_owned(), Value::Object(arguments));
    root.insert(PREVIEW.to_owned(), Value::Object(preview));
    Value::Object(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods;

    /// 判据 1: **字段名与默认值逐字对齐领域侧** —— 不是人眼比对，是拿**真**领域类型与
    /// **真管线**产出的载荷对账。
    ///
    /// 注入验证（把 [`DRY_RUN_PARAM`] 改回手写字面量 `"dry_run"`）会让本判据变红
    /// （再导出的常量会立刻不等于领域侧常量）。
    #[test]
    fn dry_run_defaults_to_false_exactly_like_the_domain() {
        use yeban_mcp::tools::{DRY_RUN_PARAM as DOMAIN_PARAM, ToolCall};

        // ① 参数名：同一个常量（再导出 ⇒ 编译器保证）。
        assert_eq!(DRY_RUN_PARAM, DOMAIN_PARAM);
        assert_eq!(DRY_RUN_PARAM, "dryRun");

        // ② 四种形状的默认值/取值必须与领域侧**逐形状**相同。
        let domain = |arguments: Value| {
            ToolCall::from_params(Some(&serde_json::json!({
                "name": "yeban_query_project",
                "arguments": arguments,
            })))
            .expect("合法的工具调用")
            .is_dry_run()
        };
        for (shape, expected) in [
            (serde_json::json!({}), false),
            (serde_json::json!({"dryRun": false}), false),
            (serde_json::json!({"dryRun": true}), true),
            (serde_json::json!({"limit": 5}), false),
        ] {
            let params = shape.as_object().expect("对象").clone();
            assert_eq!(
                requested(&params),
                domain(shape.clone()),
                "形状 {shape} 的 dryRun 取值必须与领域侧一致"
            );
            assert_eq!(requested(&params), expected, "形状 {shape}");
        }
    }

    /// 判据 2: 响应载荷里的共享字段名与领域侧**真管线**产出的 dryRun 载荷逐字相同。
    ///
    /// `yeban_mcp::samples::dry_run_response_sample()` 走的是**真实分发管线**
    /// （`Dispatcher::handle_line` → `dry_run_result`），不是手写的形状。
    ///
    /// 注入验证（把 `STATE_UNCHANGED` 改成 `"state_unchanged"`）会让本判据变红。
    #[test]
    fn payload_carries_every_domain_field_name_verbatim() {
        let sample = yeban_mcp::samples::dry_run_response_sample();
        let data = sample
            .get("data")
            .and_then(Value::as_object)
            .expect("领域侧 dryRun 载荷在 `data` 下");
        for field in [
            DRY_RUN_FLAG,
            STATE_UNCHANGED,
            WOULD_CHANGE_STATE,
            REQUIRED_SCOPE,
            ARGUMENTS,
            PREVIEW,
        ] {
            assert!(
                data.contains_key(field),
                "领域侧真实 dryRun 载荷里没有字段 `{field}`（键: {:?}）—— 本线的词表漂了",
                data.keys().collect::<Vec<_>>()
            );
        }
        // 旗标本体也要对得上（"只读模拟"这个意思本身）。
        assert_eq!(data[DRY_RUN_FLAG], Value::from(true));
        assert_eq!(data[STATE_UNCHANGED], Value::from(true));

        // 本线产出的载荷：共享字段一个不少，身份字段是 `method` / `specIds`。
        let spec = methods::method(methods::METHOD_FORCE_SAVE).expect("注册表里有");
        let payload = payload(spec, "force_save", Map::new(), None);
        let object = payload.as_object().expect("对象");
        for field in [
            DRY_RUN_FLAG,
            STATE_UNCHANGED,
            WOULD_CHANGE_STATE,
            REQUIRED_SCOPE,
            ARGUMENTS,
            PREVIEW,
            METHOD,
            SPEC_IDS,
        ] {
            assert!(object.contains_key(field), "本线载荷缺 `{field}`");
        }
        assert_eq!(payload[DRY_RUN_FLAG], Value::from(true));
        assert_eq!(payload[STATE_UNCHANGED], Value::from(true));
        assert_eq!(payload[WOULD_CHANGE_STATE], Value::from(true));
        assert_eq!(payload[METHOD], Value::from(methods::METHOD_FORCE_SAVE));
        assert_eq!(payload[REQUIRED_SCOPE], Value::from(spec.scope.as_str()));
        assert_eq!(payload[PREVIEW][OPERATION], Value::from("force_save"));
        assert_eq!(payload[PREVIEW][EFFECT], Value::Null, "执行面没给影响预览");
    }

    /// 判据 3: 只有**字面 `true`** 才算 dryRun；实参快照里 `dryRun` 与保留键都不出现。
    #[test]
    fn requested_is_total_and_the_snapshot_excludes_the_switch_itself() {
        assert!(!requested(&Map::new()));
        assert!(requested(
            serde_json::json!({"dryRun": true})
                .as_object()
                .expect("对象")
        ));
        // 参数校验已经钉过类型；这里钉的是"任何非 true 的 JSON 都不算 dryRun"。
        for value in [
            serde_json::json!({"dryRun": false}),
            serde_json::json!({"dryRun": null}),
            serde_json::json!({"dryRun": "true"}),
            serde_json::json!({"dryRun": 1}),
            serde_json::json!({"dry_run": true}),
        ] {
            let params = value.as_object().expect("对象").clone();
            assert!(!requested(&params), "{value} 不得被当成 dryRun");
        }

        let params = serde_json::json!({
            "view": "session",
            "dryRun": true,
            "_meta": {"trace": 1},
        })
        .as_object()
        .expect("对象")
        .clone();
        let snapshot = normalized_arguments(&params);
        assert_eq!(snapshot["view"], Value::from("session"));
        assert!(
            !snapshot.contains_key(DRY_RUN_PARAM),
            "dryRun 不是执行面语义"
        );
        assert!(!snapshot.contains_key("_meta"), "保留键不进实参快照");
        assert_eq!(snapshot.len(), 1);
    }
}
