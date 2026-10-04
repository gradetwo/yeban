//! 领域失败 → 契约错误码的**唯一**映射层 [MCP-TOOL-001..010, ADR-0001 D25]。
//!
//! ## 两种失败，两条出口（这条边界是承重的）
//!
//! | 失败类别 | 载体 | 为什么 |
//! | :--- | :--- | :--- |
//! | **领域失败**（工程锁被占、片段不存在……） | `ToolResponse{status:"error", error:{code}}` | `code` 必须落在 `schemas/mcp-tools.schema.json` 的闭合 enum 里 |
//! | **实现级状况**（渲染器未接线、工程文件读不动……） | JSON-RPC 错误对象 `-32005` 等 | `NOT_IMPLEMENTED` **不在**契约 enum 里，也不该在：它不是领域失败 |
//!
//! [`Fault`] 把这两类分开表示，[`Fault::into_result`] 是唯一的出海口。
//! 想在 `ToolResponse.error.code` 里塞一个契约外的字符串，必须**先改这个文件**。
//!
//! ## 为什么不用 `_ =>` 兜底
//!
//! [`code_for_model`] 对 [`ModelError`] 做**穷举**匹配（没有 `_` 分支）。
//! `yeban-model` 新增一个错误变体时，这里会**编译失败**而不是悄悄落进某个兜底码 ——
//! 兜底正是让"13 个领域错误码没有家"那类缺口长期潜伏的机制。

use std::io;

use serde_json::{Map, Value};

use yeban_model::ModelError;

use crate::jsonrpc::ErrorObject;
use crate::tools::{ErrorCode, ToolResponse};

/// 领域层失败的两种出口。
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// **领域失败**：走 `ToolResponse{status:"error"}`，`code` 来自契约 enum。
    Domain {
        /// 契约错误码。
        code: ErrorCode,
        /// 人话信息。
        message: String,
        /// 结构化补充。
        data: Option<Value>,
    },
    /// **实现级状况**：走 JSON-RPC 错误对象，绝不伪造成 `ToolResponse`。
    Impl {
        /// 已经组装好的 JSON-RPC 错误对象。
        error: ErrorObject,
    },
}

impl Fault {
    /// 领域失败。
    #[must_use]
    pub fn domain(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Domain {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// 领域失败 + 结构化补充。
    #[must_use]
    pub fn domain_with_data(code: ErrorCode, message: impl Into<String>, data: Value) -> Self {
        Self::Domain {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// 实现级状况。
    #[must_use]
    pub const fn implementation(error: ErrorObject) -> Self {
        Self::Impl { error }
    }

    /// 领域错误码（实现级状况为 `None`）。
    #[must_use]
    pub const fn domain_code(&self) -> Option<ErrorCode> {
        match self {
            Self::Domain { code, .. } => Some(*code),
            Self::Impl { .. } => None,
        }
    }

    /// 唯一的出海口：领域失败变成 `Ok(ToolResponse)`，实现级状况变成 `Err(ErrorObject)`。
    ///
    /// # Errors
    ///
    /// [`Fault::Impl`] 一定返回 `Err`。
    pub fn into_result(self) -> Result<Value, ErrorObject> {
        match self {
            Self::Domain {
                code,
                message,
                data,
            } => {
                let response = match data {
                    Some(data) => ToolResponse::Failure {
                        code,
                        message,
                        data: Some(data),
                    },
                    None => ToolResponse::failure(code, message),
                };
                Ok(response.to_value())
            }
            Self::Impl { error } => Err(error),
        }
    }

    /// 契约形状的 `ToolResponse` 错误值（实现级状况返回 `None`）。
    #[must_use]
    pub fn to_tool_response(&self) -> Option<Value> {
        match self {
            Self::Domain { .. } => self.clone().into_result().ok(),
            Self::Impl { .. } => None,
        }
    }
}

/// 实现级状况 + 结构化 `data` 的简写。
#[must_use]
pub fn not_wired(tool: &str, spec_id: &str, detail: &str, data: Map<String, Value>) -> Fault {
    let mut payload = data;
    payload.insert(
        "code".to_owned(),
        Value::from(ErrorCode::NotImplemented.as_str()),
    );
    payload.insert("tool".to_owned(), Value::from(tool));
    payload.insert("specId".to_owned(), Value::from(spec_id));
    payload.insert("detail".to_owned(), Value::from(detail));
    Fault::implementation(
        ErrorObject::new(
            crate::jsonrpc::NOT_IMPLEMENTED,
            format!("工具 `{tool}` 的这一半尚未接线"),
        )
        .with_data(Value::Object(payload)),
    )
}

/// [`ModelError`] → 契约错误码（**穷举**，没有兜底分支）。
///
/// 分组理由（每组的成员共享同一个"用户可理解"的处置）：
///
/// - 数值/音域越界 → `OUT_OF_RANGE`；
/// - 索引越界（宏 / 设备插槽 / 参数下标） → `INDEX_OUT_OF_BOUNDS`；
/// - 身份不存在 → `TRACK_NOT_FOUND` / `CLIP_NOT_FOUND` / `ENTITY_NOT_FOUND`；
/// - 载荷与文档状态不一致、键值不匹配、重复身份 → `CONFLICT`；
/// - 参数形状不合法（哈希/ULID/非有限值/采样率集合） → `INVALID_PARAMETER_RANGE`。
#[must_use]
pub fn code_for_model(error: &ModelError) -> ErrorCode {
    match error {
        ModelError::TrackNotFound { .. } => ErrorCode::TrackNotFound,
        ModelError::ClipNotFound { .. } | ModelError::ClipContentKindMismatch { .. } => {
            ErrorCode::ClipNotFound
        }
        ModelError::MacroIndexOutOfRange { .. }
        | ModelError::DeviceSlotOutOfRange { .. }
        | ModelError::ParamIndexOutOfRange { .. } => ErrorCode::IndexOutOfBounds,
        ModelError::PitchOutOfRange { .. }
        | ModelError::VelocityOutOfRange { .. }
        | ModelError::ProbabilityOutOfRange { .. }
        | ModelError::RatchetOutOfRange { .. }
        | ModelError::MicroTimingOutOfRange { .. }
        | ModelError::ZeroDuration
        | ModelError::BpmOutOfRange { .. }
        | ModelError::TimeSignatureNumeratorOutOfRange { .. }
        | ModelError::TimeSignatureDenominatorUnsupported { .. }
        | ModelError::PanOutOfRange { .. }
        | ModelError::MacroValueOutOfRange { .. }
        | ModelError::MacroDepthOutOfRange { .. } => ErrorCode::OutOfRange,
        ModelError::NoteNotFound { .. }
        | ModelError::ClipPlacementNotFound { .. }
        | ModelError::RoutingEdgeNotFound { .. }
        | ModelError::RoutingNodeNotFound { .. }
        | ModelError::AutomationPointNotFound { .. }
        | ModelError::SectionNotFound { .. }
        | ModelError::SceneNotFound { .. }
        | ModelError::CommitNotFound { .. }
        | ModelError::BranchNotFound { .. } => ErrorCode::EntityNotFound,
        ModelError::SampleRateUnsupported { .. }
        | ModelError::BlockSizeUnsupported { .. }
        | ModelError::InvalidHash { .. }
        | ModelError::InvalidEntityId { .. }
        | ModelError::NonFiniteValue { .. } => ErrorCode::InvalidParameterRange,
        ModelError::SchemaVersionTooNew { .. }
        | ModelError::ReaderTooOld { .. }
        | ModelError::DuplicateEntityId { .. }
        | ModelError::EntityKeyMismatch { .. }
        | ModelError::AssetKeyMismatch { .. }
        | ModelError::AutomationLaneTargetMismatch { .. }
        | ModelError::MasterBusKindMismatch { .. }
        | ModelError::OpStateMismatch { .. }
        | ModelError::AutomationTargetNotApplicable { .. } => ErrorCode::Conflict,
    }
}

/// [`ModelError`] → [`Fault`]（领域失败）。
#[must_use]
pub fn from_model(context: &str, error: &ModelError) -> Fault {
    Fault::domain_with_data(
        code_for_model(error),
        format!("{context}: {error}"),
        serde_json::json!({ "model": format!("{error:?}") }),
    )
}

/// `std::io::Error` → 契约错误码。
///
/// | 条件 | 码 | 判据 |
/// | :--- | :--- | :--- |
/// | `StorageFull` / `QuotaExceeded` / `ENOSPC` | `DISK_FULL` | `disk_full_kinds_map_to_disk_full`（用合成 `io::Error` 钉住，真填满磁盘做不到） |
/// | `NotFound` | `FILE_NOT_FOUND` | `open_missing_file_is_file_not_found` |
/// | 其它（含 `PermissionDenied` / `ReadOnlyFilesystem`） | `IO_ERROR` | `save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original` |
///
/// `ENOSPC` 在 Linux 与 macOS 上都是 `28`，因此这条兜底是跨平台的；
/// 主判据仍然是 `ErrorKind`（`io_error_more` 自 Rust 1.83 起稳定）。
#[must_use]
pub fn code_for_io(error: &io::Error) -> ErrorCode {
    match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => ErrorCode::DiskFull,
        io::ErrorKind::NotFound => ErrorCode::FileNotFound,
        _ if error.raw_os_error() == Some(28) => ErrorCode::DiskFull,
        _ => ErrorCode::IoError,
    }
}

/// `std::io::Error` → [`Fault`]（领域失败；`NotFound` 也走 `FILE_NOT_FOUND`）。
#[must_use]
pub fn from_io(context: &str, error: &io::Error) -> Fault {
    Fault::domain_with_data(
        code_for_io(error),
        format!("{context}: {error}"),
        serde_json::json!({ "kind": format!("{:?}", error.kind()), "osError": error.raw_os_error() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_errors_never_escape_the_contract_enum() {
        // 每个 ModelError 变体都必须映射到**契约 enum 之内**的码。
        let samples: Vec<ModelError> = vec![
            ModelError::ZeroDuration,
            ModelError::SampleRateUnsupported { value: 7 },
            ModelError::TrackNotFound {
                id: yeban_model::EntityId::default(),
            },
            ModelError::ClipNotFound {
                id: yeban_model::EntityId::default(),
            },
            ModelError::MacroIndexOutOfRange { index: 3, len: 1 },
            ModelError::PitchOutOfRange { value: 200 },
            ModelError::NonFiniteValue {
                field: "bpm",
                value: f64::NAN,
            },
            ModelError::OpStateMismatch { op: "AddNote" },
            ModelError::ReaderTooOld {
                required: 9,
                actual: 1,
            },
        ];
        for error in &samples {
            let code = code_for_model(error);
            assert!(
                code.is_schema_contract(),
                "{error:?} 映射到了契约外的码 {code}"
            );
        }
    }

    #[test]
    fn disk_full_kinds_map_to_disk_full() {
        let full = io::Error::from(io::ErrorKind::StorageFull);
        assert_eq!(code_for_io(&full), ErrorCode::DiskFull);
        let quota = io::Error::from(io::ErrorKind::QuotaExceeded);
        assert_eq!(code_for_io(&quota), ErrorCode::DiskFull);
        // 原始 ENOSPC(28): Linux 与 macOS 同号, 因此这条兜底跨平台。
        let enospc = io::Error::from_raw_os_error(28);
        assert_eq!(code_for_io(&enospc), ErrorCode::DiskFull);
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(code_for_io(&missing), ErrorCode::FileNotFound);
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(code_for_io(&denied), ErrorCode::IoError);
    }

    #[test]
    fn implementation_faults_never_produce_a_tool_response() {
        let fault = not_wired("yeban_render_master", "MCP-TOOL-008", "x", Map::new());
        assert!(fault.domain_code().is_none());
        assert!(fault.to_tool_response().is_none());
        let error = fault.into_result().expect_err("必须走 JSON-RPC 出口");
        assert_eq!(error.code, crate::jsonrpc::NOT_IMPLEMENTED);
        assert_eq!(error.data.expect("data")["code"], "NOT_IMPLEMENTED");
    }

    #[test]
    fn domain_faults_are_in_band_and_carry_the_code() {
        let fault = Fault::domain(ErrorCode::NoActiveProject, "没有活跃工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::NoActiveProject));
        let value = fault.into_result().expect("领域失败是带内成功响应");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "NO_ACTIVE_PROJECT");
    }
}
