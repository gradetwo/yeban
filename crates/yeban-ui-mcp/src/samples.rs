//! 规范样本导出（跨语言契约对账的**非测试**入口）[MUST-GATE-010, TEST-SPEC-005]。
//!
//! 与 `yeban-mcp::samples` 同一套做法：样本由 **Rust serde** 写出，再由
//! `python3 scripts/gates/validate_schemas.py --samples-dir <dir>` 交给 **Python jsonschema**
//! 逐份对账。
//!
//! ## 本 crate 的样本是**文档样本**，而且必须如此（`.meta.` 约定）
//!
//! `schemas/mcp-tools.schema.json` 的根是 `oneOf($ref ToolCall, $ref ToolResponse)`
//! （ADR-0001 D25），而 `ToolCall` 的 `name` 是**领域**十个 `yeban_*` 工具的闭合 enum。
//! UI 控制面的线上契约是 **JSON-RPC 2.0**（`{"jsonrpc","id","result"|"error"}`），
//! `schemas/` 里**没有**它的 schema，也不该有（`schemas/**` 由集成者独占）。
//!
//! 于是只有两条路：
//!
//! | 处置 | 结果 |
//! | :--- | :--- |
//! | 把方法注册表/安全矩阵包成 `{"status":"success","data":…}` | **错**：这正是 `line/mcp-core` notes §2 **M12** 记录的"类型错误"，集成者在 `5319041` 明确纠正过（清单/快照**本来就不是**契约实例） |
//! | 用 `.meta.` 命名约定，把它们登记为**文档样本** | **对**：`validate_schemas.py` 显式 `[skip]`，且脚本侧守卫要求"同一前缀至少有一份**真实例**" |
//!
//! 本线取第二条，并且**复用既有的 `mcp-tools` 前缀**（`ui-mcp` 前缀没有对应 schema，
//! 新增前缀会被脚本判为"没有对应 schema 约定"而变红）。因此：
//!
//! - 本 crate 导出的 3 份样本**全部**是 `.meta.` 文档样本；
//! - 该前缀的**真实例**由 `yeban-mcp` 的导出提供（CI 的 `checks` 腿**先**跑
//!   `export_mcp_samples`，**再**跑本线的 `export_ui_samples`）；
//! - 若只导出本线样本就去做对账，脚本会**响亮地**报
//!   "前缀 `mcp-tools` 只有 .meta. 文档样本" —— 这条依赖被判据
//!   `ui_samples_alone_leave_the_prefix_without_a_real_instance` 钉住，
//!   不是一句口头约定。
//! - 需要集成者做的那一行见 `docs/ledger/ui-mcp-notes.md` 的 needs（把
//!   `cargo run -p yeban-ui-mcp --locked --example export_ui_samples -- --out target/schema-samples`
//!   加在 `export_mcp_samples` **之后**）。
//!
//! ## 导出什么（3 份文档样本）
//!
//! | 文件 | 内容 | 为什么值得机器可读 |
//! | :--- | :--- | :--- |
//! | `mcp-tools.ui-methods.meta.json` | 14 条方法的名字 / scope / 参数 / 规范 ID / 出处签名 | "规范里写了什么"与"线上叫什么"可逐字对账；领域契约的 `tools/list` 看不到 UI 方法集 |
//! | `mcp-tools.ui-security.meta.json` | 六级 scope × 方法矩阵、生产硬禁的**判定顺序**、本线新增的三个错误码 | `ARCH-SEC-002` / `MUST-GATE-009` 的安全矩阵是**声明**，这份样本让它可被 diff |
//! | `mcp-tools.ui-tree-projection.meta.json` | 由**真实投影函数**在确定性夹具上产出的控件树 JSON | `[UI-TEST-001]`/`[UI-MCP-002]` 的线格式（含 `visible: null` 的证据语义）可被外部工具直接消费 |

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

use crate::methods;
use crate::service::SERVICE_NAME;
use crate::tree::{TreeSource, UiTree};

/// 默认样本目录名（相对工作区 `target/`）。
pub const SAMPLES_DIR_NAME: &str = "schema-samples";

/// 文件名前缀。**刻意复用既有的 `mcp-tools`**（见模块文档）。
pub const FILE_PREFIX: &str = "mcp-tools";

/// 方法注册表文档样本。
pub const METHODS_FILE: &str = "mcp-tools.ui-methods.meta.json";

/// 安全矩阵文档样本。
pub const SECURITY_FILE: &str = "mcp-tools.ui-security.meta.json";

/// 控件树投影文档样本。
pub const TREE_FILE: &str = "mcp-tools.ui-tree-projection.meta.json";

/// 对账脚本（相对仓库根）。
pub const RECONCILE_SCRIPT: &str = "scripts/gates/validate_schemas.py";

/// 样本导出失败。
#[derive(Debug, thiserror::Error)]
pub enum SampleExportError {
    /// 文件系统错误。
    #[error("写入样本失败: {0}")]
    Io(#[from] std::io::Error),
    /// JSON 序列化错误。
    #[error("序列化样本失败: {0}")]
    Serialize(#[from] serde_json::Error),
    /// 样本自身不满足约定（写盘前的 Rust 侧自检）。
    #[error("样本 `{file}` 未通过自检: {detail}")]
    InvalidSample {
        /// 文件名。
        file: String,
        /// 人话说明。
        detail: String,
    },
}

/// 全部样本文件名（顺序固定）。
#[must_use]
pub fn sample_file_names() -> Vec<String> {
    vec![
        METHODS_FILE.to_owned(),
        SECURITY_FILE.to_owned(),
        TREE_FILE.to_owned(),
    ]
}

/// 方法注册表文档：`ui/methods` 载荷的离线快照。
#[must_use]
pub fn methods_document() -> Value {
    let mut root = Map::new();
    root.insert("kind".to_owned(), Value::from("ui-mcp.method-registry"));
    root.insert("service".to_owned(), Value::from(SERVICE_NAME));
    root.insert(
        "specIds".to_owned(),
        Value::Array(
            [
                "ARCH-UI-004",
                "MCP-DUAL-001",
                "UI-MCP-001",
                "UI-MCP-002",
                "UI-MCP-003",
                "UI-TEST-001",
                "UI-TEST-002",
            ]
            .into_iter()
            .map(Value::from)
            .collect(),
        ),
    );
    root.insert("methodCount".to_owned(), Value::from(methods::METHOD_COUNT));
    root.insert(
        "reservedParamPrefix".to_owned(),
        Value::from(methods::RESERVED_PARAM_PREFIX.to_string()),
    );
    root.insert("catalogue".to_owned(), methods::catalogue());
    root.insert(
        "note".to_owned(),
        Value::from(
            "文档样本(.meta.): UI 控制面的线上契约是 JSON-RPC 2.0, 而 schemas/ 里只有领域 \
             十个 yeban_* 工具的契约 (schemas/mcp-tools.schema.json 的根 oneOf(ToolCall, \
             ToolResponse))。把这份清单包成 ToolResponse 属于类型错误 (见 docs/ledger/\
             mcp-core-notes.md §2 M12)。",
        ),
    );
    Value::Object(root)
}

/// 安全矩阵文档：scope × 方法、判定顺序、错误码。
#[must_use]
pub fn security_document() -> Value {
    let scopes = yeban_mcp::security::Scope::ALL
        .into_iter()
        .map(|scope| {
            let mut entry = Map::new();
            entry.insert("scope".to_owned(), Value::from(scope.as_str()));
            entry.insert(
                "family".to_owned(),
                Value::from(if scope.is_ui() { "ui" } else { "app" }),
            );
            entry.insert(
                "productionForbidden".to_owned(),
                Value::from(scope.is_production_forbidden()),
            );
            entry.insert(
                "methods".to_owned(),
                Value::Array(
                    methods::METHODS
                        .iter()
                        .filter(|spec| spec.scope == scope)
                        .map(|spec| Value::from(spec.name))
                        .collect(),
                ),
            );
            Value::Object(entry)
        })
        .collect::<Vec<_>>();

    let error_codes = [
        (
            crate::service::ELEMENT_NOT_FOUND,
            "ELEMENT_NOT_FOUND",
            "语义 ID 不在控件树里",
            400,
        ),
        (
            crate::service::CAPTURE_FAILED,
            "CAPTURE_FAILED",
            "Tier-1 抓帧 / PNG 编码失败 (含 MUST-GATE-015 的尺寸非零且非全黑门槛)",
            500,
        ),
        (
            crate::service::GEOMETRY_UNAVAILABLE,
            "GEOMETRY_UNAVAILABLE",
            "元素存在但没有几何包围盒, [UI-MCP-002] 的遮罩无从执行",
            400,
        ),
    ]
    .into_iter()
    .map(|(code, name, meaning, status)| {
        let mut entry = Map::new();
        entry.insert("code".to_owned(), Value::from(code));
        entry.insert("name".to_owned(), Value::from(name));
        entry.insert("meaning".to_owned(), Value::from(meaning));
        entry.insert("httpStatus".to_owned(), Value::from(status));
        entry.insert("source".to_owned(), Value::from("yeban-ui-mcp (本线新增)"));
        Value::Object(entry)
    })
    .collect::<Vec<_>>();

    let mut root = Map::new();
    root.insert("kind".to_owned(), Value::from("ui-mcp.security-matrix"));
    root.insert("service".to_owned(), Value::from(SERVICE_NAME));
    root.insert(
        "specIds".to_owned(),
        Value::Array(
            [
                "ARCH-SEC-002",
                "ARCH-UI-004",
                "MUST-GATE-009",
                "MUST-GATE-010",
                "UI-MCP-001",
            ]
            .into_iter()
            .map(Value::from)
            .collect(),
        ),
    );
    root.insert("scopes".to_owned(), Value::Array(scopes));
    root.insert(
        "authorizationOrder".to_owned(),
        Value::Array(
            [
                "production-hard-deny (ui:inject 在生产模式, 403, 不依赖 token)",
                "token (缺/形状非法/不匹配 → 401)",
                "scope (yeban_mcp::security::ScopeSet::grants → 403)",
            ]
            .into_iter()
            .map(Value::from)
            .collect(),
        ),
    );
    root.insert(
        "defaults".to_owned(),
        serde_json::json!({
            "httpTransport": "disabled (编译期 feature `ui-mcp-http` 不在 default, 且运行期还要 --enable-ui-mcp-http)",
            "bindAddress": "127.0.0.1",
            "port": 0,
            "tokenFile": "~/.yeban/session.token (POSIX 0600, 目录 0700)",
            "injectInProduction": "denied",
            "endpointPath": crate::transport::UI_MCP_PATH,
        }),
    );
    root.insert(
        "reusedJsonRpcCodes".to_owned(),
        serde_json::json!([
            {"code": yeban_mcp::jsonrpc::PARSE_ERROR, "name": "PARSE_ERROR"},
            {"code": yeban_mcp::jsonrpc::INVALID_REQUEST, "name": "INVALID_REQUEST"},
            {"code": yeban_mcp::jsonrpc::METHOD_NOT_FOUND, "name": "METHOD_NOT_FOUND"},
            {"code": yeban_mcp::jsonrpc::INVALID_PARAMS, "name": "INVALID_PARAMS"},
            {"code": yeban_mcp::jsonrpc::INTERNAL_ERROR, "name": "INTERNAL_ERROR"},
            {"code": yeban_mcp::jsonrpc::UNAUTHORIZED, "name": "UNAUTHORIZED"},
            {"code": yeban_mcp::jsonrpc::FORBIDDEN, "name": "FORBIDDEN"},
            {"code": yeban_mcp::jsonrpc::NOT_IMPLEMENTED, "name": "NOT_IMPLEMENTED"},
        ]),
    );
    root.insert("errorCodes".to_owned(), Value::Array(error_codes));
    Value::Object(root)
}

/// 控件树投影文档：由**真实投影函数**产出的线格式快照。
///
/// 夹具是确定性的（无时间、无随机、无窗口），因此这份样本逐字节稳定。
#[must_use]
pub fn tree_projection_document() -> Value {
    let mut tree = yeban_ui_test_port::tree::ControlTree::new();
    let fixtures = [
        ("track-0-fader", "slider", "轨道 0 推子", None, false),
        (
            "arrangement-playhead",
            "image",
            "走带光标",
            Some(yeban_ui_test_port::image::Rect::new(300, 0, 2, 600)),
            true,
        ),
        (
            "transport-timecode",
            "text",
            "时间码",
            Some(yeban_ui_test_port::image::Rect::new(8, 4, 96, 16)),
            true,
        ),
        (
            "clip-01J8ZQ9K2M-header",
            "list-item",
            "剪辑头",
            Some(yeban_ui_test_port::image::Rect::new(40, 60, 120, 24)),
            false,
        ),
    ];
    for (id, role, label, bounds, dynamic) in fixtures {
        let mut node = yeban_ui_test_port::tree::ControlNode::new(
            id,
            yeban_ui_test_port::tree::Role::parse(role).expect("夹具角色合法"),
            label,
        );
        if let Some(rect) = bounds {
            node = node.with_bounds(rect);
        }
        if dynamic {
            node = node.as_dynamic();
        }
        tree.insert(node).expect("插入夹具节点");
    }
    let projection = UiTree::from_runtime(&tree);
    assert_eq!(projection.source, TreeSource::Runtime, "夹具是运行时来源");
    let mut root = Map::new();
    root.insert("kind".to_owned(), Value::from("ui-mcp.tree-projection"));
    root.insert("service".to_owned(), Value::from(SERVICE_NAME));
    root.insert(
        "specIds".to_owned(),
        Value::Array(
            ["UI-TEST-001", "UI-MCP-001", "UI-MCP-002"]
                .into_iter()
                .map(Value::from)
                .collect(),
        ),
    );
    root.insert(
        "note".to_owned(),
        Value::from(
            "文档样本(.meta.): 这是 ui/tree 的线格式, 不是领域契约实例。`visible` 为 null \
             表示**不可断定**(没有几何证据), 本投影永不输出 false, 见 src/tree.rs 的模块文档。",
        ),
    );
    root.insert(
        "tree".to_owned(),
        serde_json::to_value(&projection).unwrap_or(Value::Null),
    );
    Value::Object(root)
}

/// 默认样本目录：`<repo>/target/schema-samples`。
///
/// 遵循 Cargo 的 `CARGO_TARGET_DIR`（若设置），否则用 `<crate>/../../target`。
#[must_use]
pub fn default_out_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || manifest_dir.join("..").join("..").join("target"),
        PathBuf::from,
    );
    target.join(SAMPLES_DIR_NAME)
}

/// 仓库根目录（`CARGO_MANIFEST_DIR/../..`）。
#[must_use]
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 把 3 份**文档样本**写到 `out_dir`，返回实际写出的路径（顺序固定）。
///
/// 写盘**之前**先做 Rust 侧自检（[`check_document_sample`]）——磁盘上不会出现
/// 一份连自己都不合约定的样本。
///
/// # Errors
///
/// 目录创建 / 写入失败、序列化失败，或任一样本未通过自检。
pub fn export_all(out_dir: &Path) -> Result<Vec<PathBuf>, SampleExportError> {
    std::fs::create_dir_all(out_dir)?;
    let mut written = Vec::new();
    for (file, sample) in [
        (METHODS_FILE, methods_document()),
        (SECURITY_FILE, security_document()),
        (TREE_FILE, tree_projection_document()),
    ] {
        check_document_sample(file, &sample)?;
        written.push(write_json(out_dir, file, &sample)?);
    }
    Ok(written)
}

/// 写到 [`default_out_dir`]。
///
/// # Errors
///
/// 同 [`export_all`]。
pub fn export_to_default_dir() -> Result<Vec<PathBuf>, SampleExportError> {
    export_all(&default_out_dir())
}

/// 文档样本的 Rust 侧自检。
///
/// 两道：
///
/// 1. 名字里必须真的含 `.meta.`（否则对账脚本会拿 `oneOf` 根去校验一份清单 —— 类型错误）；
/// 2. 复用 `yeban_mcp::samples::check_document_sample` 的**同一套**判别键规则
///    （顶层不许出现 `name` / `arguments` / `status`）—— 依赖 `yeban-mcp` 而不是重写，
///    免得"`.meta.` 藏实例"的缺口在一侧被堵上、另一侧还开着；
/// 3. 本线再加一条：`kind` 必须是 `ui-mcp.*`，让"这份文档是谁的"机器可读。
///
/// # Errors
///
/// 违反上述任一条。
pub fn check_document_sample(file: &str, sample: &Value) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    if !file.contains(".meta.") {
        return Err(invalid(
            "本 crate 只导出文档样本, 文件名必须含 `.meta.` (见 samples.rs 模块文档)".to_owned(),
        ));
    }
    yeban_mcp::samples::check_document_sample(file, sample)
        .map_err(|error| invalid(error.to_string()))?;
    let kind = sample
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !kind.starts_with("ui-mcp.") {
        return Err(invalid(format!(
            "文档样本的 `kind` 必须形如 `ui-mcp.*`, 实际 `{kind}`"
        )));
    }
    Ok(())
}

/// 把一行证据**直写进程 fd 2**（绕过 libtest 的输出捕获）。
///
/// 为什么需要它：`cargo test` 会捕获通过测试的 `println!`/`eprintln!`，
/// 于是"跨语言对账真的跑了、结论是什么"在 CI 日志里**什么都看不到** ——
/// 一条"数字型/判决型"门禁的判决必须看得见（做法与
/// `yeban_ui_test_port::render::report_line` 一致）。
pub fn report_line(line: &str) {
    use std::io::Write as _;
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

/// 跨语言对账的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconcile {
    /// 脚本退出码 0。
    Passed {
        /// 脚本输出（尾部若干行）。
        detail: String,
    },
    /// 脚本退出码非 0（且不是"缺依赖"）。
    Failed {
        /// 脚本输出（尾部若干行）。
        detail: String,
    },
    /// **响亮跳过**：环境里没有 `python3` / `jsonschema`，或脚本不存在。
    ///
    /// 不是"通过"——调用方必须把 `detail` 打出来，让跳过这件事看得见。
    Skipped {
        /// 为什么跳过。
        detail: String,
    },
}

impl Reconcile {
    /// 是否真的通过了跨语言对账。
    #[must_use]
    pub const fn is_passed(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }

    /// 输出 / 原因。
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Passed { detail } | Self::Failed { detail } | Self::Skipped { detail } => detail,
        }
    }
}

/// 跑 `scripts/gates/validate_schemas.py --samples-dir <dir>`。
///
/// 缺 `python3` / `jsonschema` 时返回 [`Reconcile::Skipped`]（**响亮**，不是静默通过）——
/// 与 `yeban-mcp` 的做法一致（`docs/ledger/mcp-core-notes.md` §3.2 的诚实边界 (a)）。
#[must_use]
pub fn reconcile_with_schemas(dir: &Path) -> Reconcile {
    let script = repo_root().join(RECONCILE_SCRIPT);
    if !script.is_file() {
        return Reconcile::Skipped {
            detail: format!("找不到对账脚本: {}", script.display()),
        };
    }
    let output = Command::new("python3")
        .arg(&script)
        .arg("--samples-dir")
        .arg(dir)
        .current_dir(repo_root())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Reconcile::Skipped {
                detail: format!(
                    "LOUD SKIP: 无法执行 python3 ({error}); 跨语言契约对账在本机**没有跑**"
                ),
            };
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    match output.status.code() {
        Some(0) => Reconcile::Passed {
            detail: tail(&stdout, 6),
        },
        // 脚本用 2 表示"缺 jsonschema 依赖"（见 validate_schemas.py 的 ImportError 分支）。
        Some(2) if combined.contains("jsonschema") => Reconcile::Skipped {
            detail: format!(
                "LOUD SKIP: 环境里没有 python3/jsonschema, 跨语言契约对账**没有跑**。\n{}",
                tail(&combined, 4)
            ),
        },
        _ => Reconcile::Failed {
            detail: format!("退出码 {:?}\n{}", output.status.code(), tail(&combined, 12)),
        },
    }
}

/// 取文本尾部若干行（日志进判据时不要把整段输出塞进去）。
fn tail(text: &str, lines: usize) -> String {
    let all = text.lines().collect::<Vec<_>>();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// 以"美化 JSON + 结尾换行"写出一个样本。
fn write_json(out_dir: &Path, file: &str, value: &Value) -> Result<PathBuf, SampleExportError> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    let path = out_dir.join(file);
    std::fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yeban-ui-mcp-samples-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read_dir(dir: &Path) -> Vec<(String, String)> {
        let mut entries = std::fs::read_dir(dir)
            .expect("读目录")
            .map(|entry| {
                let entry = entry.expect("目录项");
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(entry.path()).expect("读文件"),
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    /// 判据 1（**双射守卫**，与 mcp-core notes §M13 同形）: 本 crate 导出的文件名集合
    /// **恰好**是那三份 `.meta.` 文档样本 —— 少一份、多一份、去掉 `.meta.` 都会红。
    ///
    /// 注入验证 E（把一份文档改名成不带 `.meta.` 的"实例"）会让本判据变红。
    #[test]
    fn exported_sample_set_is_exactly_the_three_documents() {
        let dir = temp_dir("set");
        let written = export_all(&dir).expect("导出");
        let names = written
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("文件名")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, sample_file_names());
        assert_eq!(
            names,
            vec![
                "mcp-tools.ui-methods.meta.json",
                "mcp-tools.ui-security.meta.json",
                "mcp-tools.ui-tree-projection.meta.json"
            ]
        );
        for name in &names {
            assert!(
                name.starts_with(FILE_PREFIX),
                "{name} 必须复用 mcp-tools 前缀"
            );
            assert!(name.contains(".meta."), "{name} 必须是文档样本");
        }
        assert_eq!(read_dir(&dir).len(), names.len(), "目录里不许有别的文件");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 2: 文档样本**不许**长得像契约实例（顶层判别键 + `kind` 归属）。
    #[test]
    fn documents_do_not_look_like_contract_instances() {
        for (file, sample) in [
            (METHODS_FILE, methods_document()),
            (SECURITY_FILE, security_document()),
            (TREE_FILE, tree_projection_document()),
        ] {
            check_document_sample(file, &sample).expect("合法文档样本");
            let object = sample.as_object().expect("对象");
            for forbidden in ["name", "arguments", "status"] {
                assert!(
                    !object.contains_key(forbidden),
                    "`{file}` 顶层不得出现契约实例判别键 `{forbidden}`"
                );
            }

            // 反例 1: 去掉 `.meta.` ⇒ 必须被拒（否则会被 `oneOf` 根校验，属于类型错误）。
            let renamed = file.replace(".meta.", ".");
            assert!(
                check_document_sample(&renamed, &sample).is_err(),
                "不带 .meta. 的名字必须被拒: {renamed}"
            );
            // 反例 2: 顶层塞一个 `status`（伪装成 ToolResponse）⇒ 必须被拒。
            let mut disguised = object.clone();
            disguised.insert("status".to_owned(), Value::from("success"));
            assert!(
                check_document_sample(file, &Value::Object(disguised)).is_err(),
                "`{file}` 伪装成 ToolResponse 必须被拒"
            );
            // 反例 3: `kind` 不是本 crate 的。
            let mut foreign = object.clone();
            foreign.insert("kind".to_owned(), Value::from("mcp-tools.registry"));
            assert!(check_document_sample(file, &Value::Object(foreign)).is_err());
        }
    }

    /// 判据 3: 两次导出**逐字节相同**（样本要能当 diff 基准）。
    #[test]
    fn export_is_byte_stable() {
        let first = temp_dir("stable-a");
        let second = temp_dir("stable-b");
        export_all(&first).expect("导出 A");
        export_all(&second).expect("导出 B");
        let left = read_dir(&first);
        let right = read_dir(&second);
        assert_eq!(left, right, "两次导出的字节必须完全相同");
        assert!(left.iter().all(|(_, text)| text.ends_with("}\n")));
        let tree = left
            .iter()
            .find(|(name, _)| name == TREE_FILE)
            .expect("有树样本");
        let json: Value = serde_json::from_str(&tree.1).expect("JSON");
        assert_eq!(json["tree"]["source"], "runtime");
        assert_eq!(json["tree"]["count"], 4);
        assert_eq!(
            json["tree"]["nodes"][0]["id"], "arrangement-playhead",
            "节点必须按语义 ID 升序"
        );
        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    /// 判据 4: 方法 / 安全文档必须与**实现**逐条对得上（不是手抄的清单）。
    #[test]
    fn documents_match_the_implementation() {
        let methods = methods_document();
        let listed = methods["catalogue"]["methods"].as_array().expect("数组");
        assert_eq!(listed.len(), methods::METHOD_COUNT);
        assert_eq!(methods["methodCount"], methods::METHOD_COUNT);
        for (spec, entry) in methods::METHODS.iter().zip(listed) {
            assert_eq!(entry["requiredScope"], spec.scope.as_str());
            assert_eq!(
                entry["specIds"].as_array().expect("数组").len(),
                spec.spec_ids.len()
            );
        }

        let security = security_document();
        let scopes = security["scopes"].as_array().expect("数组");
        assert_eq!(scopes.len(), 6, "六级 scope 一个都不能少");
        let total_methods: usize = scopes
            .iter()
            .map(|entry| entry["methods"].as_array().expect("数组").len())
            .sum();
        assert_eq!(
            total_methods,
            methods::METHOD_COUNT,
            "每条方法恰好挂一个 scope"
        );
        let inject = scopes
            .iter()
            .find(|entry| entry["scope"] == "ui:inject")
            .expect("有 ui:inject");
        assert_eq!(inject["productionForbidden"], true);
        assert_eq!(
            inject["methods"].as_array().expect("数组").len(),
            5,
            "`ui:inject` 一族 = §12.4 的四个注入方法 + `ADR-0004` S1 的 `ui/set_track_height`"
        );
        assert_eq!(security["defaults"]["bindAddress"], "127.0.0.1");
        assert_eq!(security["defaults"]["injectInProduction"], "denied");
        assert_eq!(
            security["defaults"]["endpointPath"],
            crate::transport::UI_MCP_PATH
        );
    }

    /// 判据 5（**承重**）: 把本线样本与 `yeban-mcp` 的**真实例**放进同一个目录，
    /// 跨语言对账（Python `jsonschema`）必须通过；混进一份故意违法的实例必须变红。
    ///
    /// 缺 `jsonschema` 时**响亮 SKIP**（不伪装成通过）。
    #[test]
    fn reconciliation_with_the_domain_contract_instances() {
        let dir = temp_dir("reconcile");
        // 真实例来自领域 MCP 的导出（本 crate 依赖 yeban-mcp, 所以能直接调）。
        yeban_mcp::samples::export_all(&dir).expect("导出领域样本");
        export_all(&dir).expect("导出本线样本");

        let verdict = reconcile_with_schemas(&dir);
        report_line(&format!(
            "[yeban-ui-mcp] 跨语言契约对账 (本线 3 份 .meta. + 领域真实例): {}",
            verdict.detail().replace('\n', " | ")
        ));
        match &verdict {
            Reconcile::Passed { detail } => {
                assert!(
                    detail.contains("契约校验通过"),
                    "通过时应当打印脚本结论: {detail}"
                );
            }
            Reconcile::Skipped { detail } => {
                // 响亮 SKIP: 判据不算通过, 但也**不伪造红**。
                eprintln!("!!! LOG SKIP: 跨语言契约对账没有跑 !!!\n{detail}");
            }
            Reconcile::Failed { detail } => {
                panic!("本线样本 + 领域真实例必须通过根 oneOf 对账:\n{detail}");
            }
        }

        // 负向对照: 混进一份**故意违法**的实例 ⇒ 必须变红（证明这条判据是承重的）。
        if !matches!(verdict, Reconcile::Skipped { .. }) {
            let bogus = dir.join("mcp-tools.bogus.json");
            std::fs::write(&bogus, "{\"anything\":[1,2,3]}\n").expect("写违法样本");
            let verdict = reconcile_with_schemas(&dir);
            assert!(
                matches!(verdict, Reconcile::Failed { .. }),
                "故意违法的样本必须让对账变红, 实际: {verdict:?}"
            );
            std::fs::remove_file(&bogus).ok();
            assert!(
                reconcile_with_schemas(&dir).is_passed()
                    || matches!(reconcile_with_schemas(&dir), Reconcile::Skipped { .. }),
                "删掉违法样本后必须回到绿"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 6: **单独**导出本线样本会让 `mcp-tools` 前缀只剩 `.meta.` —— 脚本会响亮地红。
    ///
    /// 这条判据把"必须与 `yeban-mcp` 的导出一起对账"从一句口头约定变成**可执行的依赖**：
    /// 它断言的是**脚本的行为**，不是我们的希望。
    #[test]
    fn ui_samples_alone_leave_the_prefix_without_a_real_instance() {
        let dir = temp_dir("meta-only");
        export_all(&dir).expect("只导出本线样本");
        let verdict = reconcile_with_schemas(&dir);
        report_line(&format!(
            "[yeban-ui-mcp] 只导出本线样本时的脚本结论 (期望红): {}",
            verdict.detail().replace('\n', " | ")
        ));
        match verdict {
            Reconcile::Skipped { detail } => {
                eprintln!("!!! LOG SKIP: 跨语言契约对账没有跑 !!!\n{detail}");
            }
            Reconcile::Failed { detail } => {
                assert!(
                    detail.contains("只有 .meta."),
                    "红的原因必须是前缀守卫: {detail}"
                );
            }
            Reconcile::Passed { detail } => {
                panic!("只有 .meta. 文档样本时不该通过 —— 那说明脚本的前缀守卫失效了:\n{detail}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 7: 默认目录落在工作区 `target/schema-samples`（`export_ui_samples` 的 `--out` 默认值）。
    #[test]
    fn default_out_dir_is_the_workspace_target() {
        let dir = default_out_dir();
        assert!(dir.ends_with(SAMPLES_DIR_NAME));
        assert!(
            dir.to_string_lossy().contains("target"),
            "默认目录必须在 target 下: {}",
            dir.display()
        );
        assert!(repo_root().join("schemas").is_dir());
    }
}
