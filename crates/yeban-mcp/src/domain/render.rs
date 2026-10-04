//! `yeban_render_master` 的参数校验与 **scope**（渲染本体明确未接线）[MCP-TOOL-008]。
//!
//! ## 哪一半是真的
//!
//! - `format` 白名单（`wav` / `rf64` / `bw64`）；
//! - `sampleRate` 走 [`yeban_model::SampleRate::from_hz`] —— 也就是**模型层**的
//!   允许集合 `{44100,48000,88200,96000,192000}`，不是这里另抄一份；
//! - `normalize` 缺省 `false`；
//! - 无活跃工程 → `NO_ACTIVE_PROJECT`；
//! - `dryRun` → 真的做上面全部校验，并**如实披露**渲染器未接线。
//!
//! ## 哪一半没有
//!
//! 触发 `yeban-render` 的多核并行导出、返回内容哈希与产物路径 —— 这一半**没有接线**
//! （离线渲染是 `line/render-master` 的资产，本线不改其它 crate）。
//! 因此真调用在**通过全部参数校验之后**返回 JSON-RPC `-32005 NOT_IMPLEMENTED`
//! （`ADR-0001`/`mcp-core` M10：实现级状况绝不伪造成 `ToolResponse`）。
//!
//! 判据 `render_master_validates_before_it_reports_not_wired` 钉住"校验先于未接线"：
//! 坏参数必须拿到 `ToolResponse` 的领域错误码，好参数才拿到 `-32005`。
//!
//! ## `BUSY` 为什么不可达
//!
//! §7.2 给本工具声明了 `BUSY`（"引擎正忙"）。本 crate 的领域状态是**单线程同步**的：
//! 一次 `tools/call` 完整跑完才返回，不存在"已经有一个渲染在跑"的窗口。
//! 因此 `BUSY` 在当前架构下**不可达** —— 这是登记，不是遗漏
//! （接线到真正的异步渲染管线时它才会变成可达码）。

use serde_json::{Map, Value};

use yeban_model::{SampleRate, YebanProjectV1};

use super::error::Fault;
use crate::tools::ErrorCode;

/// 允许的输出容器/编码。
pub const FORMATS: [&str; 3] = ["wav", "rf64", "bw64"];

/// 一次已校验的渲染请求（**还没有渲染**）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRequest {
    /// 输出格式。
    pub format_index: usize,
    /// 采样率（模型层枚举）。
    pub sample_rate: SampleRate,
    /// 是否归一化。
    pub normalize: bool,
}

impl RenderRequest {
    /// 输出格式字符串。
    #[must_use]
    pub fn format(self) -> &'static str {
        FORMATS.get(self.format_index).copied().unwrap_or("wav")
    }

    /// `dryRun` 预览用的 JSON。
    #[must_use]
    pub fn preview(self) -> Value {
        serde_json::json!({
            "format": self.format(),
            "sampleRate": self.sample_rate.hz(),
            "normalize": self.normalize,
            "renderer": "yeban-render",
            "wired": false,
            "wiredReason": "离线渲染是 line/render-master 的资产; 本线不改其它 crate",
            "willReturn": "JSON-RPC -32005 NOT_IMPLEMENTED (字段校验已通过)",
        })
    }
}

/// 校验渲染参数。
///
/// # Errors
///
/// - 未知 `format` → `INVALID_PARAMETER_RANGE`（带白名单）；
/// - `sampleRate` 不在模型层允许集合 → `INVALID_PARAMETER_RANGE`；
/// - `normalize` 不是布尔（契约的参数校验已拦，这里是第二道）。
pub fn validate(arguments: &Map<String, Value>) -> Result<RenderRequest, Fault> {
    let format = arguments
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let format_index = FORMATS
        .iter()
        .position(|known| *known == format)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("不支持的输出格式 `{format}`"),
                serde_json::json!({ "supportedFormats": FORMATS }),
            )
        })?;
    let sample_rate_hz = arguments
        .get("sampleRate")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`sampleRate` 必须是 32 位无符号整数",
            )
        })?;
    let sample_rate = SampleRate::from_hz(sample_rate_hz)
        .map_err(|error| super::error::from_model("采样率校验", &error))?;
    let normalize = match arguments.get("normalize") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`normalize` 必须是布尔值, 实际收到 {other}"),
            ));
        }
    };
    Ok(RenderRequest {
        format_index,
        sample_rate,
        normalize,
    })
}

/// 渲染请求的"工程前提"检查（无活跃工程 → `NO_ACTIVE_PROJECT`）。
///
/// 单独成函数是为了让"参数校验"与"会话前提"两条判据可以分别变红。
///
/// # Errors
///
/// 没有活跃工程。
pub fn require_project(project: Option<&YebanProjectV1>) -> Result<(), Fault> {
    if project.is_none() {
        return Err(Fault::domain(
            ErrorCode::NoActiveProject,
            "没有活跃工程, 无法渲染母带",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn unknown_format_and_sample_rate_are_invalid_parameter_range() {
        let fault = validate(&args(
            serde_json::json!({"format": "mp3", "sampleRate": 48000}),
        ))
        .expect_err("未知格式");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE");
        assert!(value["error"]["data"]["supportedFormats"].is_array());

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 12345}),
        ))
        .expect_err("采样率集合外");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": -1}),
        ))
        .expect_err("负数采样率");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
    }

    #[test]
    fn valid_arguments_produce_a_checked_request() {
        let request = validate(&args(
            serde_json::json!({"format": "rf64", "sampleRate": 96000, "normalize": true}),
        ))
        .expect("合法");
        assert_eq!(request.format(), "rf64");
        assert_eq!(request.sample_rate.hz(), 96000);
        assert!(request.normalize);
        // normalize 缺省为 false。
        let default = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 44100}),
        ))
        .expect("合法");
        assert!(!default.normalize);
        assert_eq!(default.preview()["wired"], false);
    }

    #[test]
    fn missing_project_is_no_active_project() {
        let fault = require_project(None).expect_err("没有工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::NoActiveProject));
        let project = YebanProjectV1::default();
        require_project(Some(&project)).expect("有工程");
    }
}
