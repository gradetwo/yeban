//! T-ISS-MWXK1GWW：广播 schema 与登记 schema 的同步护栏。
//!
//! `tools/list` 的 `inputSchema` 由注册表派生（`crate::tools::ToolSpec::input_schema`），
//! 而 `schemas/mcp-tools.schema.json` 的 `RegisteredToolArguments.$defs` 是**手写副本**。
//! 两边曾各自更新（T-FIX-732 的 `device` 必填集 / `insertedDeviceIds` 回显），
//! 若只改一边，运行期广播与离线契约就会漂移而套件仍绿。本判据对结构骨架逐工具对账。

use serde_json::{Map, Value};

/// 仓库根的登记 schema。
fn registered_schema() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/mcp-tools.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()));
    serde_json::from_str(&text).expect("schema 必须是合法 JSON")
}

/// 结构骨架：保留约束与形状键，剥掉叙述性文字（`description` / `title`）
/// 与生成侧不会写的元数据（`x-*`）；`required` 按字典序比，避免纯声明顺序漂移。
fn skeleton(value: &Value) -> Value {
    const SHAPE_KEYS: &[&str] = &[
        "type",
        "required",
        "properties",
        "items",
        "oneOf",
        "anyOf",
        "allOf",
        "enum",
        "const",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
    ];
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for key in SHAPE_KEYS {
                if let Some(value) = map.get(*key) {
                    out.insert((*key).to_owned(), skeleton(value));
                }
            }
            // 静态 extension 条目常省略 `required`（= 无可必填键），生成侧恒发
            // 或干脆不发；统一成数组缺席即空集，再按字典序排，避免纯声明顺序漂移。
            let required = out
                .entry("required".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(items) = required {
                items.sort_by(|left, right| {
                    left.as_str()
                        .unwrap_or_default()
                        .cmp(right.as_str().unwrap_or_default())
                });
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(skeleton).collect()),
        other => other.clone(),
    }
}

#[test]
fn registered_argument_schemas_match_the_runtime_broadcast() {
    let registered = registered_schema();
    // 两段共 46 个工具（10 个 documented + 36 个 extension）。
    // `DocumentedToolArguments.$defs` 只登记可选键的子集，不是完整参数形状，故不比对。
    let mut checked = 0;
    for section in ["RegisteredToolArguments", "ExtensionToolArguments"] {
        let defs = registered["definitions"][section]["$defs"]
            .as_object()
            .unwrap_or_else(|| panic!("{section}.$defs 必须是对象"));
        for (name, expected) in defs {
            let spec = yeban_mcp::tools::tool(name)
                .unwrap_or_else(|| panic!("登记 schema 里的工具 `{name}` 不在运行时注册表"));
            let actual = spec.input_schema();
            assert_eq!(
                skeleton(&actual),
                skeleton(expected),
                "`{name}` 的 tools/list inputSchema 与 schemas/mcp-tools.schema.json 漂移；\
                 修改工具参数时必须同步改登记 schema（或反过来）"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        yeban_mcp::tools::tool_names().len(),
        "静态 schema 的工具条目数必须与运行时注册表一致"
    );
}

/// ⭐ T-FIX-956：`yeban_edit_automation` 的 `point` / `ticks` 必须广播嵌套约束，
/// 且上界与条数护栏直接读运行时常量（⛔ 不手抄 256 / u64::MAX）。
#[test]
fn automation_point_and_ticks_broadcast_nested_constraints() {
    let spec = yeban_mcp::tools::tool("yeban_edit_automation")
        .expect("edit_automation 必须在运行时注册表");
    let schema = spec.input_schema();
    let max_tick = u64::MAX;

    let point = &schema["properties"]["point"];
    assert_eq!(point["type"], "object");
    assert_eq!(point["properties"]["tick"]["type"], "integer");
    assert_eq!(point["properties"]["tick"]["minimum"], 0_u64);
    assert_eq!(point["properties"]["tick"]["maximum"], max_tick);
    assert_eq!(point["properties"]["value"]["type"], "number");
    assert_eq!(
        point["properties"]["curve"]["enum"],
        serde_json::json!(yeban_mcp::domain::extension_pure::CURVE_NAMES)
    );
    let required = point["required"].as_array().expect("point.required");
    assert!(
        required.iter().any(|v| v == "tick"),
        "point.required 缺 tick"
    );
    assert!(
        required.iter().any(|v| v == "value"),
        "point.required 缺 value"
    );

    let ticks = &schema["properties"]["ticks"];
    assert_eq!(ticks["type"], "array");
    assert_eq!(ticks["items"]["type"], "integer");
    assert_eq!(ticks["items"]["minimum"], 0_u64);
    assert_eq!(ticks["items"]["maximum"], max_tick);
    assert_eq!(
        ticks["maxItems"],
        serde_json::json!(yeban_mcp::domain::automation::MAX_READ_TICKS)
    );

    // 红对照：把 tools.rs 的 `extra_schema` 广播分支摘掉 ⇒ point.properties / ticks.items 缺席，
    // 本判据在 point.properties.tick.type 处红。
}
