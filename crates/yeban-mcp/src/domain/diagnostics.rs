//! `[D56]` `yeban_export_diagnostics` 的实现 —— 把调试信息与相关文件采集并导出成 zip 诊断包。
//!
//! 两阶段契约照 `import_audio`（`plan` 校验 + 组装，`apply` 执行副作用）：
//! - [`plan`] **不**需要活跃工程：出问题时常常连工程都打不开，日志与环境信息恰恰最该能采。
//! - [`apply`] 调 `yeban_diagnostics::export_diagnostics`（**同一实现**，D56 判据 4）。
//!
//! 隐私默认：`project/` 为空即不含工程；`config.json` 在本版本里**写明不可用**而不是留空（不留"看起来有"）。
use serde_json::{Map, Value, json};

use crate::tools::ErrorCode;

use super::error::Fault;
use super::{ToolResponse, require_active};

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

    // 会话/引擎快照：有活跃工程就序列化它；没有就**什么都不写**（不伪造状态）。
    let state_json: Option<String> = match require_active(domain) {
        Ok(project) => serde_json::to_string(project).ok(),
        Err(_) => None,
    };

    // 本机配置层：本版本**没有**把它暴露给 MCP，故写一条明确的"不可用"记录。
    // 判据要求 `config.json` 在场；写"不可用"是诚实的在场，留空或省略都不是。
    let config_json = json!({
        "note": "config layer is not exposed to MCP in this build",
        "source": "unavailable",
    })
    .to_string();

    let logs: Vec<(String, Vec<u8>)> = Vec::new();
    let crashes: Vec<(String, Vec<u8>)> = Vec::new();
    // 隐私默认：不含工程文件。用户要带工程时应走 UI 的勾选项，而不是这个默认路径。
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
        "projectIncluded": false,
    })))
}
