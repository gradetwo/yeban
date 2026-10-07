//! `[D56]` `yeban_export_diagnostics` 的实现 —— 把调试信息与相关文件采集并导出成 zip 诊断包。
//!
//! 两阶段契约照 `import_audio`（`plan` 校验 + 组装，`apply` 执行副作用）：
//! - [`plan`] **不**需要活跃工程：出问题时常常连工程都打不开，日志与环境信息恰恰最该能采。
//! - [`apply`] 调 `yeban_diagnostics::export_diagnostics`（**同一实现**，D56 判据 4）。
//!
//! 隐私默认：**整个包**都不含工程文档 —— `project/` 为空即不含工程文件，而
//! `engine-state.json` 也只承载 [`super::engine_state`] 的**投影**（走带 / 采样率 /
//! 缓冲 / 响度 / 撤销游标），**不是** `YebanProjectV1` 的序列化。`config.json` 在本版本里
//! **写明不可用**而不是留空（不留"看起来有"）。
//!
//! 为什么把这条写进模块文档而不是只写一句注释：诊断包的用途是"贴进 issue 复现"，
//! 拿到包的人**看不进 zip**，只能相信响应里的 `projectIncluded`。所以"包内字节"与
//! "响应 flag"是同一件事的两个面，必须逐字一致 —— 判据
//! `diagnostics_bundle_content_matches_the_project_included_flag`（`tests/extension_tools.rs`）
//! 就是钉这条的。
use serde_json::{Map, Value, json};

use crate::tools::ErrorCode;

use super::error::Fault;
use super::{ToolResponse, engine_state, require_active};

/// `plan` 的产物：已校验的输出目录 + 要在 `apply` 阶段采集的内容。
#[derive(Debug, Clone)]
pub struct DiagnosticsExport {
    /// 输出目录；`None` 表示用**系统临时目录**（不写当前目录，免得污染仓库）。
    pub out_dir: Option<std::path::PathBuf>,
}

/// 校验入参。**不**读取工程，因此无需 `require_active`。
///
/// # Errors
/// `outDir` 存在但不是字符串、或指向一个**存在的文件**（而非目录）时返回 [`Fault`]。
pub fn plan(arguments: &Map<String, Value>) -> Result<DiagnosticsExport, Fault> {
    let out_dir = match arguments.get("outDir") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => {
            let path = std::path::PathBuf::from(s);
            if path.is_file() {
                return Err(Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`outDir` 指向一个已存在的**文件**；需要目录",
                ));
            }
            Some(path)
        }
        Some(_) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`outDir` 必须是字符串",
            ));
        }
    };
    Ok(DiagnosticsExport { out_dir })
}

/// 执行导出。**这是唯一产生副作用的地方**（写一个独立 zip，不改工程状态 ⇒ `side_effect = ReadOnly`）。
///
/// # Errors
/// 目录不可创建、或打包失败时返回 [`Fault`]。
pub fn apply(
    domain: &mut super::Domain,
    planned: &DiagnosticsExport,
) -> Result<ToolResponse, Fault> {
    // 缺省写到**系统临时目录**, 不写进程当前目录: 后者会让任何"无参调用"的测试
    // 把 zip 产物落进仓库（实测: 两条测试各留一个包在 crate 目录里）。
    // 响应里始终给出完整路径, 所以人/agent 仍然找得到它。
    let out_dir = match &planned.out_dir {
        Some(p) => p.clone(),
        None => std::env::temp_dir(),
    };
    if !out_dir.is_dir() {
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            Fault::domain(
                ErrorCode::IoError,
                format!("创建输出目录失败 {}: {e}", out_dir.display()),
            )
        })?;
    }

    // 引擎/会话快照：有活跃工程就投影它；没有就**什么都不写**（不伪造状态）。
    //
    // ⛔ 这里**不许**写 `serde_json::to_string(project)`。那是 `YebanProjectV1` 的**完整文档**
    // （tracks / clip_pool / 音符 / 自动化 / assets 索引），而工程文档有**自己**的通道
    // `BundleInputs::project`（D56 判据 6：只在用户显式勾选时才非空，默认不含 = 隐私决定）。
    // 曾把整个工程塞进 `state_json`，于是包内 `engine-state.json` 与保存容器的 `project.json`
    // **逐字节相同**，而响应仍写 `projectIncluded:false` —— flag 与字节互相矛盾。
    // 拿这个包去贴 issue 的人看不进 zip，只能相信 flag：这正是最坏的失败形态。
    // 现在的快照走**既有投影** [`engine_state::snapshot`]（D56 判据 4 的"既有投影即可"），
    // 与 `yeban_query_engine_state` 同一份拼装，不引入第二份状态表示。
    let state_json: Option<String> = match require_active(domain) {
        Ok(project) => {
            // `track_id = None` ⇒ 投影里 `track: null`，不点名任何音轨（点名也是工程内容）。
            let snapshot = engine_state::snapshot(
                project,
                domain.session(),
                domain.engine_readings(),
                domain.undo_state().undone(),
                None,
                engine_state::ReadingsCursor {
                    revision: domain.readings_revision(),
                    tail: domain.readings_tail(),
                    since: None,
                },
            );
            // `track_id = None` 时 `snapshot` 唯一的失败态（音轨不存在）不可达；序列化失败
            // 也只是"这次没有快照"，不该让整包导出失败（包里的 env/git/config 仍然有用）。
            snapshot
                .ok()
                .and_then(|value| serde_json::to_string(&value).ok())
        }
        Err(_) => None,
    };

    // 本机配置层：本版本**没有**把它暴露给 MCP，故写一条明确的"不可用"记录。
    // 判据要求 `config.json` 在场；写"不可用"是诚实的在场，留空或省略都不是。
    // 与 UI 动作共用**同一**函数（D56 判据 4）。
    let config_json = yeban_diagnostics::unavailable_config_json();

    let logs: Vec<(String, Vec<u8>)> = Vec::new();
    let crashes: Vec<(String, Vec<u8>)> = Vec::new();
    // 隐私默认：不含工程文件。用户要带工程时应走 UI 的勾选项，而不是这个默认路径；
    // MCP 侧**没有**"带工程"的开关，所以这里恒为空，响应里的 `projectIncluded` 因此恒为 false。
    let project_files: Vec<(String, Vec<u8>)> = Vec::new();

    let inputs = yeban_diagnostics::BundleInputs {
        state_json: state_json.as_deref(),
        config_json: Some(&config_json),
        logs: &logs,
        crashes: &crashes,
        project: &project_files,
    };
    let report = yeban_diagnostics::export_diagnostics(&out_dir, inputs)
        .map_err(|e| Fault::domain(ErrorCode::IoError, format!("导出诊断包失败: {e}")))?;

    let entries: Vec<Value> = report
        .entries
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "bytes": entry.bytes,
                "sha256": entry.sha256,
            })
        })
        .collect();

    Ok(ToolResponse::success(json!({
        "path": report.path.display().to_string(),
        "bytes": report.bytes,
        "sha256": report.sha256,
        "entries": entries,
        // 口径：`true` ⇔ 包内**真的有**工程文档（`project/` 条目或 `engine-state.json` 里的
        // 工程文档）。本工具没有"带工程"的入参，`project/` 恒空、`engine-state.json` 恒为
        // 引擎/会话**投影** ⇒ 恒 `false`。判据 `diagnostics_bundle_content_matches_the_project_included_flag`
        // 逐条读回包内字节来钉这个不变量（flag 与字节只要有一个动了就必须红）。
        "projectIncluded": false,
    })))
}
