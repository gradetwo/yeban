//! 驱动错误：`OtcpError` 是本 crate 唯一的失败类型。
//!
//! 每个错误带一个 MCP 面的 `reason`（固定注册表，见 `REASONS`）——
//! MCP 适配层（`yeban-mcp/src/domain/otcp.rs`）按 `reason` 一一映射成
//! `Fault`，⛔ 不解析 `message` 文本。`data` 是结构化上下文（任务 id、
//! 失败块、音符间隙、自测记录……），同样不靠文本解析。
//!
//! OTCP 服务端错误（`error.data.kind` / `reason`）到驱动 `reason` 的
//! 映射见 [`OtcpError::from_otcp`]（设计稿 §4.1 的错误分类表）；
//! 映射只看 `kind` 与 `reason`，⛔ 不看 `detail` 文本。

use serde_json::{Value, json};

/// MCP 面的固定 `reason` 注册表（本 crate 产出的全部取值）。
///
/// ⚠ 新增取值时必须同步三处：`yeban-mcp` 的 `GOLDEN_REASONS` /
/// `GOLDEN_REASON_PAYLOADS`（`tests/fault_vocabulary.rs`）与
/// `tests/otcp_tools.rs` 里对该取值的断言；缺一处即红。
pub const REASONS: &[&str] = &[
    "otcpRenderFailed",
    "otcpModalBlocked",
    "otcpDialogCommits",
    "otcpRevisionConflict",
    "otcpNotReady",
    "otcpContentGone",
    "otcpMeasureFailed",
    "otcpNoEditor",
    "otcpUnsupportedBuild",
    "otcpDoubleCorrection",
    "otcpStereoUnsupported",
    "otcpServerGone",
    "otcpBusy",
    "otcpJobNotFound",
    "otcpNotesUnavailable",
    "otcpNotConfigured",
    "otcpSourceNotWav",
    "otcpSourceLicenseMissing",
    "otcpTimeGridUnsupported",
];

/// 驱动错误。
#[derive(Clone, Debug, PartialEq)]
pub struct OtcpError {
    /// MCP 面的固定 `reason`（见 [`REASONS`]）。
    pub reason: &'static str,
    /// 人读的说明（⛔ 不作机器判定依据）。
    pub message: String,
    /// 结构化上下文（任务 id、失败块、音符间隙、自测记录……）。
    pub data: Value,
}

impl OtcpError {
    /// 新建一个无上下文的错误。
    #[must_use]
    pub fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
            data: Value::Null,
        }
    }

    /// 新建一个带结构化上下文的错误。
    #[must_use]
    pub fn with_data(reason: &'static str, message: impl Into<String>, data: Value) -> Self {
        Self {
            reason,
            message: message.into(),
            data,
        }
    }

    /// 从 OTCP 服务端错误对象映射（设计稿 §4.1）。
    ///
    /// 只看 `kind` 与 `reason`，⛔ 不看 `detail` 文本。
    /// `job_id` 为空时不写进 `data`。
    #[must_use]
    pub fn from_otcp(kind: &str, reason: &str, detail: &str, job_id: &str) -> Self {
        let mut data = json!({
            "otcpKind": kind,
            "otcpReason": reason,
            "detail": detail,
        });
        if !job_id.is_empty() {
            data["job_id"] = Value::String(job_id.to_owned());
        }
        let message = format!("OpenTune 报错 {kind}({reason}): {detail}");
        match (kind, reason) {
            ("RENDER_FAILED", _) => Self::with_data("otcpRenderFailed", message, data),
            ("NOT_READY", "f0_failed" | "f0_empty") => {
                Self::with_data("otcpNotReady", message, data)
            }
            ("NOT_READY", "export_dry_stretched") => {
                Self::with_data("otcpTimeGridUnsupported", message, data)
            }
            ("NOT_READY", _) => Self::with_data("otcpNotReady", message, data),
            ("ANALYSIS_FAILED", _) => Self::with_data("otcpNotReady", message, data),
            ("REVISION_CONFLICT", _) => Self::with_data("otcpRevisionConflict", message, data),
            ("BUSY", "modal_open") => Self::with_data("otcpModalBlocked", message, data),
            ("BUSY", _) => Self::with_data("otcpBusy", message, data),
            ("NOT_FOUND", _) => Self::with_data("otcpContentGone", message, data),
            ("UNSUPPORTED", "no_editor_window") => Self::with_data("otcpNoEditor", message, data),
            ("UNSUPPORTED", _) => Self::with_data("otcpUnsupportedBuild", message, data),
            ("SHUTTING_DOWN", _) => Self::with_data("otcpServerGone", message, data),
            ("INVALID_ARGUMENT", "dialog_commits_on_close") => {
                Self::with_data("otcpDialogCommits", message, data)
            }
            ("INVALID_ARGUMENT", _)
            | ("INTERNAL", _)
            | ("TIMEOUT", _)
            | ("MODEL_UNAVAILABLE", _)
            | ("CANCELLED", _) => Self::with_data("otcpServerGone", message, data),
            _ => Self::with_data("otcpServerGone", message, data),
        }
    }

    /// 传输层失败（连接断开、超时、帧损坏）：服务端已不可信。
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self::new("otcpServerGone", message)
    }

    /// 协议层失败（握手校验不符、缺字段、形状非法）：服务端不可信，⛔ 不再发请求。
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("otcpServerGone", message)
    }

    /// 本地 IO 失败（任务目录、收据读写）。
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self::new("otcpServerGone", message)
    }
}

impl std::fmt::Display for OtcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.reason, self.message)
    }
}

impl std::error::Error for OtcpError {}

impl From<std::io::Error> for OtcpError {
    fn from(error: std::io::Error) -> Self {
        Self::transport(format!("IO 失败: {error}"))
    }
}

/// OTCP JSON-RPC 错误对象（`{code, message, data: {kind, reason, ...}}`）。
///
/// 供 [`OtcpError::from_otcp`] 之外的调用点复用：解析一次，处处同口径。
#[derive(Clone, Debug, PartialEq)]
pub struct OtcpRpcError {
    /// JSON-RPC 错误码。
    pub code: i64,
    /// `data.kind`（与 `message` 相等，见 spec）。
    pub kind: String,
    /// `data.reason`。
    pub reason: String,
    /// `data.retryable`（缺省 `false`）。
    pub retryable: bool,
    /// `data.detail`（人读文本，⛔ 不作判定依据）。
    pub detail: String,
    /// `data.executed`（缺省 `false`）。
    pub executed: bool,
    /// `data.retryAfterMs`（有则透传，供轮询遵守；缺省 `None`）。
    pub retry_after_ms: Option<u64>,
}

impl OtcpRpcError {
    /// 从 JSON-RPC 错误对象解析；形状非法 ⇒ `None`。
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        let data = value.get("data")?;
        Some(Self {
            code: value.get("code")?.as_i64()?,
            kind: data.get("kind")?.as_str()?.to_owned(),
            reason: data
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            retryable: data
                .get("retryable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            detail: data
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            executed: data
                .get("executed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            retry_after_ms: data.get("retryAfterMs").and_then(Value::as_u64),
        })
    }

    /// 转驱动错误（带任务 id 上下文；`retryAfterMs` 有则进 `data`）。
    #[must_use]
    pub fn to_driver_error(&self, job_id: &str) -> OtcpError {
        let mut error = OtcpError::from_otcp(&self.kind, &self.reason, &self.detail, job_id);
        if let (Some(ms), Value::Object(map)) = (self.retry_after_ms, &mut error.data) {
            map.insert("retryAfterMs".to_owned(), Value::from(ms));
        }
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_failed_maps_to_loud_failure() {
        let error = OtcpError::from_otcp(
            "RENDER_FAILED",
            "render_chunk_failed",
            "a render block of a content of the export failed twice",
            "job-1",
        );
        assert_eq!(error.reason, "otcpRenderFailed");
        assert_eq!(error.data["otcpKind"], "RENDER_FAILED");
        assert_eq!(error.data["job_id"], "job-1");
    }

    #[test]
    fn not_ready_variants_map_by_reason() {
        assert_eq!(
            OtcpError::from_otcp("NOT_READY", "f0_not_ready", "", "").reason,
            "otcpNotReady"
        );
        assert_eq!(
            OtcpError::from_otcp("NOT_READY", "export_dry_stretched", "", "").reason,
            "otcpTimeGridUnsupported"
        );
        assert_eq!(
            OtcpError::from_otcp("NOT_READY", "f0_failed", "", "").reason,
            "otcpNotReady"
        );
    }

    #[test]
    fn modal_and_dialog_reasons_are_distinct() {
        assert_eq!(
            OtcpError::from_otcp("BUSY", "modal_open", "", "").reason,
            "otcpModalBlocked"
        );
        assert_eq!(
            OtcpError::from_otcp("INVALID_ARGUMENT", "dialog_commits_on_close", "", "").reason,
            "otcpDialogCommits"
        );
    }

    #[test]
    fn rpc_error_parses_and_maps() {
        let value = json!({
            "code": -32000,
            "message": "RENDER_FAILED",
            "data": {
                "kind": "RENDER_FAILED",
                "reason": "render_chunk_failed",
                "retryable": false,
                "detail": "a render block failed twice",
                "executed": false
            }
        });
        let rpc = OtcpRpcError::from_value(&value).expect("形状合法");
        assert_eq!(rpc.kind, "RENDER_FAILED");
        assert!(!rpc.retryable);
        assert!(!rpc.executed);
        assert_eq!(rpc.to_driver_error("j1").reason, "otcpRenderFailed");
    }

    #[test]
    fn rpc_error_rejects_bad_shape() {
        assert!(OtcpRpcError::from_value(&json!({"code": 1})).is_none());
        assert!(OtcpRpcError::from_value(&json!({})).is_none());
    }
}
