//! 契约分支覆盖：`schemas/project.schema.json` 的 `tracks[*].sampler`（任务单 `#81`）。
//!
//! ## 为什么需要它
//!
//! 任务单 `#76` 给模型加了 `TrackV3.sampler: Option<SamplerBinding>` 并同步了契约，
//! 但**样本刻意不带它**（[`yeban_model::samples::filled_project`] 不落盘 `sampler`，
//! 那正是"默认工程不变"的判据本身）—— 于是 `validate_schemas.py --samples-dir` 这条
//! 常驻对账**从来没有触达** `sampler` 分支：删 `required`、把 `path` 改成 `number`、
//! 给分支改名，CI 今天全都抓不到（`#76` 自己如实登记了这一点）。
//!
//! ## 判据（2 条，全部进 `cargo test -p yeban-model`）
//!
//! | # | 判据 | 需要 python? | 测的是什么 |
//! | :-- | :--- | :--- | :--- |
//! | ① | [`schema_sampler_branch_shape_matches_the_model_type`] | ⛔ 不需要 | 契约声明的形状 ⇔ `SamplerBinding` 的**真实 serde 输出**（属性名/类型/必需性），＋ 读写往返，＋ 由契约声明推导的接受/拒收 |
//! | ② | [`python_jsonschema_gate_agrees_on_the_sampler_branch`] | ⭐ 需要 | 用**既有**的 `scripts/gates/validate_schemas.py --samples-dir` 拿真 jsonschema 判正/反向 |
//!
//! ① 在**任何**环境都跑（零 python、零新依赖）；② 只在装了 `jsonschema` 的环境真跑，
//! 缺依赖时打印**响亮的 SKIP**、⛔ 不伪造绿（与 `crates/yeban-mcp/tests/contract.rs` 同一处置）。
//! ⚠ 为什么不能只留 ②：CI 的 **rust 矩阵腿不装 jsonschema**，而装了 jsonschema 的 `checks`
//! job 直调脚本、**不跑** `cargo test` ⇒ 只留 ② 就是"判据写好了但从不执行"的假绿。
//!
//! ## 复用的既有形状（⛔ 不另发明机制）
//!
//! - "读契约文件 → 与模型对账"是 `tests/schema_ratchet.rs` 的做法：那条棘轮判的是
//!   **`required` 的存在性/必需性**，明确把"类型/枚举/区间"留给 `validate_schemas.py`
//!   ⇒ 它看不见"分支的形状对不对"。本文件用同一手法（直接读契约、不手抄事实源）
//!   补上 `sampler` 这一棵子树的**形状**对账；
//! - "样本 ↔ jsonschema 判决"直接用 `scripts/gates/validate_schemas.py --samples-dir`
//!   本身，并照抄 `crates/yeban-mcp/tests/contract.rs` 的"干净样本必须过 ＋ 故意违法样本
//!   必须被点名拒收"那条形状 —— 只把样本换成带 `sampler` 的**整份工程**。
//!
//! ## 边界（诚实声明）
//!
//! - ①② 只判 `sampler` 这一棵子树（正是本单登记的覆盖面缺口）。整份工程的
//!   类型/枚举/区间由 `validate_schemas.py` 与 `tests/schema_ratchet.rs` 分别承担；
//! - 本文件**只读** `schemas/**`（不拥有它）：契约若要改，报集成者，⛔ 不改契约换绿；
//! - 判据 ② 依赖 `python3` ＋ `jsonschema`，且**在本机**会跑真的 python 进程（秒级）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yeban_model::samples::filled_project;
use yeban_model::{SamplerBinding, YebanProjectV1};

/// 仓库根（`<crate>/../../`）。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 相对仓库根的只读路径。
fn repo_path(rel: &str) -> PathBuf {
    repo_root().join(rel)
}

/// 读契约（**只读**）：本线不改 `schemas/**`。
fn load_schema() -> Value {
    let path = repo_path("schemas/project.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} 不是合法 JSON: {error}", path.display()))
}

/// `tracks[*]` 的 `sampler` 分支：`tracks` 是 `BTreeMap`，元素 schema 住在
/// `additionalProperties`（见 `schemas/project.schema.json`）。
fn sampler_spec(schema: &Value) -> &Value {
    schema
        .pointer("/properties/tracks/additionalProperties/properties/sampler")
        .expect("契约里必须存在 tracks[*].sampler 分支（任务单 #76 引入）")
}

/// `serde_json::Value` 的 JSON Schema 类型名（与 `tests/schema_ratchet.rs` 同口径）。
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// **只**实现 `sampler` 分支用到的三个关键字（`type` / `required` / `properties`）的实例判决。
///
/// ⚠ 这**不是**通用 JSON Schema 校验器，也**不**取代 `validate_schemas.py`（判据 ② 才是
/// 真 jsonschema）。它的唯一职责：让"契约声明的形状"与"模型类型写出的形状"在
/// **不依赖 python** 的环境里也能对账 —— CI 的 rust 矩阵腿不装 jsonschema。
///
/// 因此遇到本函数**不认识**的关键字一律 `panic!`（"不许静默漏判"，与
/// `tests/schema_ratchet.rs` 的 unsupported 清单同一口径）：契约一旦给 `sampler` 长出
/// `enum` / `pattern` / `$ref` / `oneOf` / `additionalProperties`，本判据当场红，
/// 逼作者显式扩展它，而不是悄悄少判一条。
fn accepts(branch: &Value, instance: &Value) -> bool {
    let object = match branch.as_object() {
        Some(object) => object,
        None => panic!("sampler 分支必须是对象"),
    };
    for (keyword, spec) in object {
        match keyword.as_str() {
            "description" | "type" | "required" | "properties" => {}
            other => panic!("sampler 分支出现本判据不支持的关键字 `{other}` —— 请显式扩展本判据"),
        }
        if keyword == "properties" {
            let properties = spec.as_object().expect("`properties` 必须是对象");
            for (name, prop) in properties {
                let prop = prop
                    .as_object()
                    .unwrap_or_else(|| panic!("`properties.{name}` 必须是对象"));
                for nested in prop.keys() {
                    assert!(
                        matches!(nested.as_str(), "type" | "description"),
                        "`properties.{name}` 出现本判据不支持的关键字 `{nested}`"
                    );
                }
            }
        }
    }

    let type_ok = match object.get("type") {
        None => true,
        Some(Value::String(name)) => json_type(instance) == name,
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| json_type(instance) == name),
        Some(_) => panic!("`type` 必须是字符串或字符串数组"),
    };
    if !type_ok {
        return false;
    }

    // 下面两个关键字只对 object 实例有意义（非 object 已被 `type` 挡掉，或就是 `null`）。
    let Some(map) = instance.as_object() else {
        return true;
    };
    if let Some(required) = object.get("required") {
        for name in required.as_array().expect("`required` 必须是数组") {
            let name = name.as_str().expect("`required` 元素必须是字符串");
            if !map.contains_key(name) {
                return false;
            }
        }
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, spec) in properties {
            if map.get(name).is_some_and(|value| !accepts(spec, value)) {
                return false;
            }
        }
    }
    true
}

/// 把一份工程的**第一条轨**的 `sampler` 设成给定值（其余字节保持 `filled_project`）。
fn document_with_sampler(sampler: Value) -> Value {
    let mut document = serde_json::to_value(filled_project()).expect("filled_project 必须可序列化");
    let track_id = document["tracks"]
        .as_object()
        .expect("tracks 是对象（BTreeMap<EntityId, TrackV3>）")
        .keys()
        .next()
        .expect("filled_project 至少有一条轨")
        .clone();
    document["tracks"][&track_id]["sampler"] = sampler;
    document
}

// ---------------------------------------------------------------------------
// ① 形状对账（零 python / 零新依赖：任何环境都跑）
// ---------------------------------------------------------------------------

/// 契约的 `sampler` 分支形状必须与 `SamplerBinding` **真实写出的形状**一致，
/// 并且用契约自己的声明判决正/反向实例。
///
/// 改坏契约的哪一处会红（注入双向实测见任务单 `#81` 的报告）：
///
/// - 删 `required` ⇒ ④ 红（必需性）＋ ⑤ 的 `assert!(!accepts(spec, &json!({})))` 失败 ⇒ 红；
/// - `path` 类型改成 `number` ⇒ ③ 红（实例是 `string`）＋ ⑤ 的 `{"path": 1}` 变成合法
///   ⇒ 反空洞断言失败 ⇒ 红；
/// - 属性改名（`path` → 别的）⇒ ② 红（与 serde 写出的键集不等）；
/// - 给分支加本判据不认识的关键字 ⇒ `accepts` panic（不许静默漏判）。
#[test]
fn schema_sampler_branch_shape_matches_the_model_type() {
    let schema = load_schema();
    let spec = sampler_spec(&schema);
    let binding = SamplerBinding {
        path: "instruments/piano.sfz".to_owned(),
    };
    let binding_json = serde_json::to_value(&binding).expect("SamplerBinding 必须可序列化");

    // ② 属性名集合：契约声明 ⇔ 模型类型写出的键（**双向**相等，事实源是 serde 输出，不手抄）。
    let declared: BTreeSet<&str> = spec["properties"]
        .as_object()
        .expect("sampler 分支必须有 `properties`")
        .keys()
        .map(String::as_str)
        .collect();
    let written: BTreeSet<&str> = binding_json
        .as_object()
        .expect("SamplerBinding 序列化为对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        declared, written,
        "契约 `sampler.properties` 的键集与 `SamplerBinding` 的真实 serde 输出不一致"
    );

    // ③ 每个字段的类型：实例值必须命中契约声明的类型。
    for (name, value) in binding_json.as_object().expect("对象") {
        assert!(
            accepts(&spec["properties"][name.as_str()], value),
            "`sampler.{name}` 的实例值 `{value}` 不满足契约声明的类型"
        );
    }

    // ④ 必需性：`SamplerBinding` 的字段是裸 `String`（无 `Option`、无 `skip_serializing_if`）
    //    ⇒ 写入器**总是**写出它们 ⇒ 契约必须把它们全部 `required`。
    //    ⚠ 若将来给本类型加了 `Option` 字段，本判据会红 —— 那是**提醒**作者去同步
    //    ADR-0001 D43 的裁决（`tests/schema_ratchet.rs` 的豁免集与 notes §5.4），
    //    ⛔ 不是放宽本判据换绿。
    let required: BTreeSet<&str> = spec["required"]
        .as_array()
        .expect("sampler 分支必须有 `required`")
        .iter()
        .map(|name| name.as_str().expect("`required` 元素必须是字符串"))
        .collect();
    assert_eq!(
        required, written,
        "契约 `sampler.required` 必须恰好是写入器总是写出的那些键"
    );

    // ⑤ 由契约**自己的声明**判决正/反向实例（不是另一份手抄的口径）。
    assert!(
        accepts(spec, &binding_json),
        "模型写出的 `{{\"path\": …}}` 必须被 sampler 分支接受"
    );
    assert!(
        accepts(spec, &Value::Null),
        "`null` 必须被接受（契约 description: null 与缺席同义）"
    );
    assert!(!accepts(spec, &json!({})), "`{{}}`（缺 `path`）必须被拒");
    assert!(!accepts(spec, &json!("x")), "`\"x\"`（类型错）必须被拒");
    // 反空洞：若 `accepts` 恒真，上面两条负向会静默通过 ⇒ 用**必然错**的字段类型证明它有牙。
    assert!(
        !accepts(spec, &json!({ "path": 1 })),
        "`{{\"path\": 1}}`（字段类型错）必须被拒 —— 否则 `accepts` 恒真，本判据空转"
    );

    // ⑥ `Option::None` ⇒ 键**缺席**（不是 `null`）：这是"默认不改变既有工程"的写侧形态。
    let absent = serde_json::to_value(filled_project()).expect("filled_project 必须可序列化");
    for (track_id, track) in absent["tracks"].as_object().expect("tracks 是对象") {
        assert!(
            track.get("sampler").is_none(),
            "轨 `{track_id}` 的 `sampler` 不该落盘（`None` ⇒ 键缺席）"
        );
    }

    // ⑦ 读写往返：带 `sampler` 的文档必须能被模型读回，且路径逐字保留。
    let parsed: YebanProjectV1 = serde_json::from_value(document_with_sampler(json!({
        "path": "instruments/piano.sfz"
    })))
    .expect("带 sampler 的文档必须可读回");
    let bindings: Vec<&SamplerBinding> = parsed
        .tracks
        .values()
        .filter_map(|track| track.sampler.as_ref())
        .collect();
    assert_eq!(bindings.len(), 1, "恰好一条轨带 sampler");
    assert_eq!(
        bindings[0].path, "instruments/piano.sfz",
        "路径必须逐字保留"
    );
}

// ---------------------------------------------------------------------------
// ② 真 jsonschema 判决（既有 validate_schemas.py；缺依赖则响亮 SKIP）
// ---------------------------------------------------------------------------

/// 用**既有**的 `scripts/gates/validate_schemas.py --samples-dir` 判决三份样本：
///
/// - 正向：整份工程 ＋ `sampler = {"path": …}`（＋ `sampler = null`）⇒ 必须通过；
/// - 反向 a：`sampler = {}`（缺 `path`）⇒ 必须被**点名**拒收；
/// - 反向 b：`sampler = "x"`（类型错）⇒ 必须被**点名**拒收。
///
/// ⚠ 这条判据需要 `python3` ＋ `jsonschema`：CI 的 **rust 矩阵腿不装**它 ⇒ 那里打印
/// 响亮的 SKIP（真跑在装了 jsonschema 的环境/本机）。Rust 矩阵腿里承重的是判据 ①。
#[test]
fn python_jsonschema_gate_agrees_on_the_sampler_branch() {
    if !python_jsonschema_available() {
        eprintln!(
            "SKIP[python_jsonschema_gate_agrees_on_the_sampler_branch]: python3/jsonschema 不可用 \
             （CI 的 rust 矩阵腿不装它）。本条不伪造绿；sampler 分支的常驻承重判据是 \
             schema_sampler_branch_shape_matches_the_model_type。"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "yeban-model-sampler-contract-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时样本目录");

    // (1) 正向：`{"path": …}` 与 `null` 两种合法形态都必须过整份 project.schema.json。
    write_sample(
        &dir,
        "project.sampler.json",
        &document_with_sampler(json!({ "path": "instruments/piano.sfz" })),
    );
    write_sample(
        &dir,
        "project.sampler.null.json",
        &document_with_sampler(Value::Null),
    );
    let clean = run_validate_schemas(&dir);
    eprintln!(
        "[sampler-branch] 正向 exit={} stdout={}",
        clean.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&clean.stdout).trim()
    );
    assert!(
        clean.status.success(),
        "带 `sampler`（及 `sampler: null`）的工程必须通过契约。stdout={}\nstderr={}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean_stdout = String::from_utf8_lossy(&clean.stdout);
    for name in ["project.sampler.json", "project.sampler.null.json"] {
        assert!(
            clean_stdout.contains(&format!("[ok] {name}: 通过 project.schema.json")),
            "`{name}` 必须被逐份点名通过: {clean_stdout}"
        );
    }

    // (2) 反向 a：`sampler = {}`（缺 `path`）⇒ 必须红，且红必须指向**这份样本**。
    let missing_path = "project.sampler.missing_path.json";
    write_sample(&dir, missing_path, &document_with_sampler(json!({})));
    let dirty = run_validate_schemas(&dir);
    let dirty_stderr = String::from_utf8_lossy(&dirty.stderr).into_owned();
    eprintln!(
        "[sampler-branch] 反向 a (`sampler: {{}}`) exit={} stderr={}",
        dirty.status.code().unwrap_or(-1),
        dirty_stderr.trim()
    );
    assert!(
        !dirty.status.success(),
        "`sampler: {{}}`（缺 `path`）必须让契约变红 —— 否则这条对账是空转的"
    );
    assert!(
        dirty_stderr.contains(missing_path),
        "变红的原因必须点名 `{missing_path}`: {dirty_stderr}"
    );

    // (3) 反向 b：`sampler = "x"`（类型错）⇒ 同样必须被点名拒收。
    std::fs::remove_file(dir.join(missing_path)).expect("移除反向 a 样本");
    let wrong_type = "project.sampler.wrong_type.json";
    write_sample(&dir, wrong_type, &document_with_sampler(json!("x")));
    let dirty = run_validate_schemas(&dir);
    let dirty_stderr = String::from_utf8_lossy(&dirty.stderr).into_owned();
    eprintln!(
        "[sampler-branch] 反向 b (`sampler: \"x\"`) exit={} stderr={}",
        dirty.status.code().unwrap_or(-1),
        dirty_stderr.trim()
    );
    assert!(
        !dirty.status.success(),
        "`sampler: \"x\"`（类型错）必须让契约变红"
    );
    assert!(
        dirty_stderr.contains(wrong_type),
        "变红的原因必须点名 `{wrong_type}`: {dirty_stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 写一份样本（`validate_schemas.py --samples-dir` 只要求合法 JSON）。
fn write_sample(dir: &Path, name: &str, document: &Value) {
    let text = serde_json::to_string_pretty(document).expect("样本必须可序列化");
    std::fs::write(dir.join(name), text)
        .unwrap_or_else(|error| panic!("写样本 {name} 失败: {error}"));
}

/// `python3 -c "import jsonschema"` 是否可用（与 `crates/yeban-mcp/tests/contract.rs` 同口径）。
fn python_jsonschema_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import jsonschema"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// 跑一次 `scripts/gates/validate_schemas.py --samples-dir <dir>`（既有契约门，本线只调用）。
fn run_validate_schemas(dir: &Path) -> std::process::Output {
    let repo = repo_root();
    let script = repo
        .join("scripts")
        .join("gates")
        .join("validate_schemas.py");
    std::process::Command::new("python3")
        .arg(script)
        .arg("--samples-dir")
        .arg(dir)
        .current_dir(repo)
        .output()
        .expect("跑 validate_schemas.py")
}
